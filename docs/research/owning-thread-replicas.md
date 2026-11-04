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

## The rule for JUCE plugins, and what nexus tells a host

nexus cannot tell whether a binary is JUCE-built, so the rule holds for every
binary. It costs nothing for a binary with one instance.

`nexus_host::binary` counts the live instances nexus made, per plugin binary:

- `live_instances(path)`: how many are alive;
- `first_thread(path)`: the thread that made the first of them, i.e. the one
  that took the count from zero. This is the thread JUCE takes as that
  binary's message thread.

`PluginInstance::binary()` and `Replica::binary()` give an instance's binary
(an in-process clack entry counts as this executable).

**The test: an editor on the main thread is safe iff
`first_thread(binary)` is the main thread.** A host gets there in one of two
ways:

- **The binary's only instance** (`live_instances == 1`) may move. Drop the
  original *before* loading its replica on the main thread, so the count
  reaches zero and the main thread becomes the first thread. That costs a gap
  in the audio. A gapless move (replica first, then the swap) leaves the
  first thread on the worker for as long as any instance lives, and
  `first_thread` says so.
- **Otherwise keep every instance of that binary on the main thread**, and
  load new ones there. Move them all there once, the same way, so the count
  passes through zero. After that, editors open in place, with no move.

The counts cover nexus's own instances only. A plugin that makes JUCE
objects outside an instance (a JUCE plugin's factory does, briefly, inside
`createInstance`) is not counted.

## What it costs

A switch is `replica()` on one thread, then `Replica::load()` and `activate()`
on the other. Measured with
`NEXUS_REAL_PLUGIN_TEST=1 cargo test -p nexus-host --test replica a_switch -- --nocapture`
(M-series Mac, debug test build, release plugin binaries):

- each figure is the median of 9 switches back and forth;
- the range is across four runs (the slowest ran right after a clippy build);
- the binary was already loaded, which is the editor case: its instance is
  alive.

| plugin | switch | `replica()` | `load()` with state | `activate()` |
|---|---|---|---|---|
| nexus.test.gain (CLAP) | 0.1 ms | 0.0 ms | 0.1 ms | 0.0 ms |
| TAL-Filter-2 CLAP | 1.3–2.8 ms | ≤ 0.1 ms | 1.3–2.8 ms | 0.0 ms |
| TAL-Filter-2 VST3 | 5.4–5.9 ms | 0.0 ms | 2.7–3.0 ms | 2.7–2.9 ms |
| Surge XT Effects CLAP | 8.7–15.2 ms | ≤ 0.1 ms | 8.7–15.0 ms | ≤ 0.1 ms |
| Surge XT Effects VST3 | 8.8–15.3 ms | ≤ 0.1 ms | 8.6–15.0 ms | 0.1–0.2 ms |

A switch costs about one instance load (plus a VST3 activation) on the
receiving thread; the state itself is negligible. So opening an editor pays
one load on the main thread, once per open, at a click. A plugin tussen
measured at 300 ms to load stalls the main thread for about that long there.
The very first instance of a binary also pays dlopen; that was not measured
here, because the test probes every binary first.

## Found on the way

- **Surge XT Effects CLAP** keeps a value set on an *inactive* instance
  (through `params.flush`) out of its state until it has processed a block:
  `param_value` says 0.3 and the saved state says 0. Any state-based copy
  loses it, a saved project included. Make the replica after a block, or
  while active.
- **Surge XT Effects CLAP** sets its empty FX slot's parameter values when
  activated, on whichever thread. The tests compare values before
  activating.
- A state round trip is not byte-stable on Surge XT, so the tests compare
  parameter values, not bytes.
- The VST3 controller's own state matters: Surge XT VST3 restores a
  parameter from it that the component state alone leaves at 0.
