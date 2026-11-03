//! CLAP output events as [`Notice`]s: what a plugin writes to `process()`'s
//! and `params.flush`'s output event list.
//!
//! [`NoticeWriter`] is the output list itself, so an event becomes a notice the
//! moment the plugin pushes it, with no event buffer in between: on the audio
//! thread it writes into the preallocated notice ring and never allocates.

use clack_host::events::io::{OutputEventBuffer, TryPushError};
use clack_host::events::spaces::CoreEventSpace;
use clack_host::events::UnknownEvent;

use crate::notice::{Notice, NoticeSink};

/// An output event list that turns parameter values and gestures into notices.
/// Every other event (note ends, MIDI out) is accepted and ignored.
pub struct NoticeWriter<'a, S: NoticeSink>(pub &'a mut S);

/// The notice a CLAP output event carries, if any.
pub fn notice_of(event: &UnknownEvent) -> Option<Notice> {
    match event.as_core_event()? {
        CoreEventSpace::ParamValue(e) => {
            Some(Notice::ParamValue { id: e.param_id()?.get(), value: e.value() })
        }
        CoreEventSpace::ParamGestureBegin(e) => Some(Notice::GestureBegin(e.param_id()?.get())),
        CoreEventSpace::ParamGestureEnd(e) => Some(Notice::GestureEnd(e.param_id()?.get())),
        _ => None,
    }
}

impl<S: NoticeSink> OutputEventBuffer for NoticeWriter<'_, S> {
    /// A full notice ring refuses the event, as CLAP lets a host do.
    fn try_push(&mut self, event: &UnknownEvent) -> Result<(), TryPushError> {
        match notice_of(event) {
            Some(notice) if !self.0.notice(notice) => Err(TryPushError::new()),
            _ => Ok(()),
        }
    }
}
