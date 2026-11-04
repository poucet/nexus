# Moving a plugin to another owning thread

2026-11-04. Why nexus 0.8 *re-creates* an instance on another thread
(`PluginInstance::replica` → `Replica::load`) instead of handing a live one
across, what the formats say, and what it costs.

The ask (tussen): non-editor work (loads, pumps, parameters) runs off the
macOS main thread, and an instance runs on the main thread only while its
editor is open. nexus's contract is one owning thread per instance from load
on: CLAP's `thread-check` answers by it, and JUCE plugins take it as their
message thread.

## What the formats allow

**CLAP** (`clap/ext/thread-check.h`, CLAP 1.2 as vendored by clap-wrapper 0.3.1):

> main-thread: … This will be the same OS thread throughout the lifetime of
> the plug-in. On macOS and Windows, this must be the thread on which gui and
> timer events are received (i.e., the main thread of the program).

So the identity of an instance's main thread cannot move, active or not.
Changing it while deactivated (option A) is still outside the spec.
Calling `[main-thread]` functions from a worker while the instance "belongs"
to the main thread (option B) is outside it too. The second sentence also
means nexus's off-main owning thread (0.6) is itself outside the letter of
the spec on macOS. That is fine for plugins that do no GUI or timer work
there, and it is why editors need the main thread.

**VST3** (Steinberg's threading model, VST 3 API documentation):

> All initialisation/de-initialisation are done in the UI Thread. All function
> exported by the plug-in are called by the host in the UI Thread with the
> exception of `IAudioProcessor→process` and `→setProcessing`.

There is one UI thread, and the API has no notion of changing it.

**JUCE on macOS** (JUCE `master`, read 2026-11-04):

- `MessageManager::MessageManager()` sets `messageThreadId` to the thread
  constructing it (`juce_MessageManager.cpp`). The manager is created when a
  binary's first JUCE object is: every VST3 component, controller and factory
  `createInstance` holds a `ScopedJuceInitialiser_GUI`, refcounted per binary
  (`numScopedInitInstances`). It is deleted when the count returns to zero.
- `setCurrentThreadAsMessageThread()` is called by the VST3 wrapper only
  under `JUCE_LINUX || JUCE_BSD` (`updateCurrentMessageThread`). On macOS
  the message thread never moves.
- The macOS message queue always runs on the main run loop
  (`juce_MessageQueue_mac.h`: `runLoop = CFRunLoopGetMain()`), whichever
  thread `messageThreadId` names.
- The editor uses `isThisTheMessageThread()` to decide how an edit reaches the
  host (`juce_audio_plugin_client_VST3.cpp`: `paramChanged`, `beginGesture`,
  `endGesture`). If the editor runs off the "message thread", there is no
  `performEdit`/`beginEdit`/`endEdit`: the host never hears editor edits.

So, for a JUCE binary, the thread that creates its first live object is its
message thread for as long as any object of that binary lives. Moving an
instance (A) leaves its editor on a thread JUCE does not think is the message
thread. A re-created instance (C) is right only if no other object of the
same binary is alive when it is made. An instance of the binary that stays
on the worker keeps JUCE's message thread there.

## The shape: re-create (C)

A `Replica` is a `Send` value that holds:

- what made the instance: its bundle path and id, or an in-process clack entry;
- its state (`save_state`);
- a VST3 edit controller's own `getState`;
- values set on an inactive VST3 instance that its state does not hold yet.

A plugin without state carries its writable parameter values instead.
`Replica::load()` on the target thread loads the plugin, restores the state
(VST3: component, then `setComponentState`, then the controller's `setState`),
and sets the values.

Each instance keeps one thread for its whole life, so CLAP and VST3 are
respected to the letter, and `thread-check` is right by construction. The
original is untouched. It keeps playing until the host has activated the
replica and swapped the processors, and is then dropped on its own thread.

## What it costs

`NEXUS_REAL_PLUGIN_TEST=1 cargo test -p nexus-host --test replica`, M-series
Mac, three runs. The plugins are JUCE-built, release binaries in a debug test
build. Each binary was already loaded once by the probe, so even the "first
instance" is warm on dlopen.

| plugin | first instance | second instance | `replica()` | `Replica::load` |
|---|---|---|---|---|
| TAL-Filter-2 CLAP | 5.6–8.9 ms | 1.4–3.7 ms | ≤ 0.1 ms | 1.2–3.2 ms |
| TAL-Filter-2 VST3 | 3.4–7.8 ms | 2.8–7.2 ms | ≤ 0.1 ms | 2.8–6.7 ms |
| Surge XT Effects CLAP | 19–33 ms | 9–110 ms | ≤ 3.4 ms | 8.7–30 ms |
| Surge XT Effects VST3 | 9–15 ms | 9–16 ms | ≤ 0.1 ms | 8.5–11 ms |

A replica costs about one instance load on the receiving thread; the state
itself is negligible. Opening an editor therefore pays one load on the main
thread, once per open, at a click. The plugins tussen measured at 300 ms
pay that 300 ms there.

## Found on the way

- **Surge XT Effects CLAP** keeps a value set on an *inactive* instance
  (through `params.flush`) out of its state until it has processed a block:
  `param_value` says 0.3 and the saved state says 0. Any state-based copy
  loses it, a saved project included. Make the replica after a block, or
  while active.
- A state round trip is not byte-stable on Surge XT, so the tests compare
  parameter values, not bytes.
- The VST3 controller's own state matters: Surge XT VST3 restores a
  parameter from it that the component state alone leaves at 0.
