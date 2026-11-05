//! Moving a plugin to another owning thread by re-creating it there.
//!
//! A [`PluginInstance`] belongs to the thread that loaded it for its whole
//! life, and the formats ask for exactly that:
//!
//! - **CLAP** (`thread-check.h`): the main thread "will be the same OS thread
//!   throughout the lifetime of the plug-in";
//! - **VST3**: every initialisation, de-initialisation and call happens on the
//!   one UI thread;
//! - **JUCE** plugins (macOS) take the thread that made their first object as
//!   their message thread, for every instance of that binary.
//!
//! So an instance is never handed across. Instead its [`Replica`] (`Send`:
//! what to load and the state to give it) crosses, and
//! [`Replica::load`] makes a new instance on the thread that wants it: a
//! plugin loaded on a worker gets a twin on the main thread before its editor
//! opens, and the editor's edits go back the same way after it closes.
//!
//! The original is untouched: it can keep playing until the host has activated
//! the replica and swapped the audio processors, and is then dropped on its
//! own thread.

use std::path::PathBuf;

use crate::host::{HostError, PluginFormat, PluginInstance};

/// What made an instance, so another thread can make it again.
#[derive(Clone)]
pub(crate) enum Origin {
    /// A plugin binary on disk, and the plugin's id in it.
    Bundle { format: PluginFormat, path: PathBuf, id: String },
    /// A CLAP entry compiled into this process
    /// ([`Host::load_clack`](crate::Host::load_clack)).
    Clack {
        load: fn(&str) -> Result<crate::clap::instance::ClapPluginInstance, HostError>,
        id: String,
    },
}

impl Origin {
    /// The plugin binary: the bundle, or this executable for an in-process
    /// entry (what [`crate::binary`] counts).
    pub(crate) fn binary(&self) -> PathBuf {
        match self {
            Origin::Bundle { path, .. } => path.clone(),
            Origin::Clack { .. } => std::env::current_exe().unwrap_or_default(),
        }
    }
}

/// A plugin instance as it is now, to be made again on another thread:
/// `Send`, unlike the instance. Made by [`PluginInstance::replica`]; turned
/// into an instance by [`load`](Self::load), on the thread that will own it.
///
/// It carries the plugin's state ([`save_state`](PluginInstance::save_state)),
/// a VST3 edit controller's own state, and values set on an inactive
/// instance that its state does not hold yet. A plugin without state carries
/// its writable parameter values instead.
///
/// Not carried: the activation (the host activates the replica and swaps the
/// audio processors), an open editor, notices not yet taken, and changes still
/// on an active instance's parameter ring (the plugin has not seen them, so
/// its state does not hold them: make the replica after the next block).
///
/// `Clone`: it is plain data (what to load, and bytes and values to give it),
/// holding no handle to the original or to any plugin. A host keeps a clone
/// to try again when [`load`](Self::load) fails, or loads one replica more
/// than once.
#[derive(Clone)]
pub struct Replica {
    origin: Origin,
    state: Option<Vec<u8>>,
    /// A VST3 edit controller's `getState`, beside the component's.
    controller: Option<Vec<u8>>,
    /// Applied after the state with `set_param`, in order.
    params: Vec<(u32, f64)>,
}

impl Replica {
    /// Read `instance` now (on its own thread). `controller` and `pending`
    /// come from the backend: a VST3 edit controller's state, and values set
    /// while inactive that the state does not hold.
    pub(crate) fn of(
        origin: Origin,
        instance: &mut dyn PluginInstance,
        controller: Option<Vec<u8>>,
        pending: Vec<(u32, f64)>,
    ) -> Self {
        let state = instance.save_state().ok();
        let params = if state.is_some() {
            pending
        } else {
            instance
                .params()
                .into_iter()
                .filter(|p| !p.is_readonly)
                .filter_map(|p| Some((p.id, instance.param_value(p.id)?)))
                .collect()
        };
        Self { origin, state, controller, params }
    }

    /// Make the instance again, owned by the calling thread: load it, give it
    /// the state, then the values. Inactive, with no editor open.
    pub fn load(self) -> Result<Box<dyn PluginInstance>, HostError> {
        use crate::clap::instance::ClapPluginInstance;
        use crate::vst3::instance::Vst3PluginInstance;
        let state = self.state.as_deref();
        let mut instance: Box<dyn PluginInstance> = match &self.origin {
            Origin::Bundle { format: PluginFormat::Clap, path, id } => {
                Box::new(restored(ClapPluginInstance::load(path, id)?, state)?)
            }
            Origin::Bundle { format: PluginFormat::Vst3, path, id } => {
                let mut vst3 = restored(Vst3PluginInstance::load(path, id)?, state)?;
                // The edit controller's own state goes in after the component's.
                if let Some(controller) = &self.controller {
                    vst3.load_controller_state(controller)?;
                }
                Box::new(vst3)
            }
            Origin::Clack { load, id } => Box::new(restored(load(id)?, state)?),
        };
        for (id, value) in self.params {
            instance.set_param(id, value)?;
        }
        Ok(instance)
    }

    /// The plugin binary the replica loads: ask
    /// [`binary::live_instances`](crate::binary::live_instances) before
    /// loading it on another thread (see [`crate::binary`]).
    pub fn binary(&self) -> PathBuf {
        self.origin.binary()
    }

    /// The plugin's state, if it has one.
    pub fn state(&self) -> Option<&[u8]> {
        self.state.as_deref()
    }
}

/// `instance` with `state` loaded, if there is one.
fn restored<P: PluginInstance>(mut instance: P, state: Option<&[u8]>) -> Result<P, HostError> {
    if let Some(state) = state {
        instance.load_state(state)?;
    }
    Ok(instance)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_send<T: Send>() {}

    #[test]
    fn a_replica_crosses_threads() {
        assert_send::<Replica>();
    }

    /// No state (and no `save_state`): two parameters, one read-only.
    struct Stateless;

    impl PluginInstance for Stateless {
        fn activate(
            &mut self,
            _: crate::AudioConfig,
        ) -> Result<Box<dyn crate::AudioProcessor>, HostError> {
            Err(HostError::Load("not in this test".into()))
        }
        fn deactivate(&mut self) {}
        fn params(&mut self) -> Vec<crate::ParamInfo> {
            [(1, false), (2, true)]
                .into_iter()
                .map(|(id, is_readonly)| crate::ParamInfo {
                    id,
                    name: format!("p{id}"),
                    module: String::new(),
                    min_value: 0.0,
                    max_value: 1.0,
                    default_value: 0.0,
                    is_stepped: false,
                    is_readonly,
                    is_hidden: false,
                    is_bypass: false,
                    is_automatable: true,
                    is_modulatable: false,
                })
                .collect()
        }
        fn param_value(&mut self, id: u32) -> Option<f64> {
            Some(f64::from(id) / 10.0)
        }
    }

    fn origin() -> Origin {
        Origin::Bundle { format: PluginFormat::Clap, path: "/nowhere".into(), id: "x".into() }
    }

    /// A plugin without state carries its writable values instead.
    #[test]
    fn a_plugin_without_state_carries_its_values() {
        let replica = Replica::of(origin(), &mut Stateless, None, Vec::new());
        assert_eq!(replica.state(), None);
        assert_eq!(replica.params, [(1, 0.1)]);
    }

    /// With state, only the values the state does not hold yet.
    #[test]
    fn with_state_only_the_pending_values_ride_along() {
        struct Stateful;
        impl PluginInstance for Stateful {
            fn activate(
                &mut self,
                _: crate::AudioConfig,
            ) -> Result<Box<dyn crate::AudioProcessor>, HostError> {
                Err(HostError::Load("not in this test".into()))
            }
            fn deactivate(&mut self) {}
            fn save_state(&mut self) -> Result<Vec<u8>, HostError> {
                Ok(vec![7])
            }
        }
        let replica = Replica::of(origin(), &mut Stateful, Some(vec![9]), vec![(4, 0.5)]);
        assert_eq!(replica.state(), Some(&[7][..]));
        assert_eq!(replica.controller.as_deref(), Some(&[9][..]));
        assert_eq!(replica.params, [(4, 0.5)]);

        // A clone carries all of it, the VST3 controller's state included
        // (there is no VST3 test plugin to load it with).
        let clone = replica.clone();
        assert_eq!(clone.state(), Some(&[7][..]));
        assert_eq!(clone.controller.as_deref(), Some(&[9][..]));
        assert_eq!(clone.params, [(4, 0.5)]);
    }
}
