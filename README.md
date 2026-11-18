# mort

*The Reaper's sidekick.*

mort lets an outside process work on a live REAPER session the way a
mix engineer does. It can set up tracks and instruments, write automation
and MIDI, play or render a range, measure what comes out, and keep a map of
what is happening in the mix at every point. Then it can change the mix and
measure again.

There are three parts:

- **`mort-bridge`**: a REAPER extension. It serves JSON-RPC on a local
  socket and runs each request against REAPER's API on the main thread.
- **`mort-probe`**: a CLAP analysis plugin, one on each track, bus and the
  master. It measures loudness, spectrum, stereo image and transients, and
  stamps each frame with the project position.
- **`mort`**: the command-line client. People and agents use it to talk to
  the bridge.

Every edit mort makes is an undo point labelled `mort: …` in REAPER, so
the person at the session can see and reverse it.

Background: silkscreen's "Fenland" piece was composed offline. The
engines were configured directly, automation was sent to them, and their
output was measured for spectral balance, masking and level. mort brings
that loop into a DAW session that is running.

Design: [adrs/](adrs/). Work: [epics/](epics/) and [tickets/](tickets/).
