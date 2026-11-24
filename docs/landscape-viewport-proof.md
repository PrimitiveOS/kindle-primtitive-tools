# Landscape viewport over the portrait framebuffer

The Oasis 3 can use a landscape drawing surface while the Kindle framebuffer stays
in its existing portrait mode. A 1680×1264 logical frame maps to the existing
1264×1680, eight-bit visible memory, using the measured 1280-byte row pitch.
[The Rust proof](../crates/kl-render/examples/landscape_portrait_buffer.rs) uses
the existing Canvas and Quick Settings renderer to exercise both directions.

## What KOReader does

KOReader revision `9192014d8bd82a91dc1012473be0f238dedfdb54` constructs the Oasis 3
framebuffer and reads its initial orientation from the firmware. The pinned base
revision `6e4bc81af4e04f78d81677a323a780a71b29702a` only sets actual framebuffer
rotation when a device opts into a rotation mechanism. On the Oasis 3 software
path, the blitbuffer gets the inverse of the requested logical turn relative to
the initial framebuffer and blitbuffer modes.
[Oasis 3 setup](https://github.com/koreader/koreader/blob/9192014d8bd82a91dc1012473be0f238dedfdb54/frontend/device/kindle/device.lua#L1641-L1705) ·
[Framebuffer selection](https://github.com/koreader/koreader-base/blob/6e4bc81af4e04f78d81677a323a780a71b29702a/ffi/framebuffer_linux.lua#L216-L239) ·
[Software rotation](https://github.com/koreader/koreader-base/blob/6e4bc81af4e04f78d81677a323a780a71b29702a/ffi/framebuffer.lua#L452-L462)

With firmware turn `F` of 0 or 2 and initial blitbuffer turn zero, the stock
logical-to-memory blitbuffer turn is `(F - A) mod 4` for requested turn `A`.
The blitbuffer projects both pixels and rectangles to memory coordinates.
The Zelda e-ink update uses the projected rectangle.
[Pixel and rectangle mapping](https://github.com/koreader/koreader-base/blob/6e4bc81af4e04f78d81677a323a780a71b29702a/ffi/blitbuffer.lua#L769-L792) ·
[Zelda update](https://github.com/koreader/koreader-base/blob/6e4bc81af4e04f78d81677a323a780a71b29702a/ffi/framebuffer_mxcfb.lua#L281-L288)

Touch follows the logical turn. Use the following mapping from an inclusive
upright native pixel at `(x, y)`, where `W` and `H` are portrait dimensions.

| Logical turn | Canvas width × height | Screen touch | Swap the existing PrimitiveOS page-key bindings |
|---|---|---|---|
| U | W × H | (x, y) | No |
| CW | H × W | (H−1−y, x) | Yes |
| D | W × H | (W−1−x, H−1−y) | Yes |
| CCW | H × W | (y, W−1−x) | No |

Transform both endpoints of swipes and holds. Firmware turn affects framebuffer
memory only. The key swap follows KOReader's relative behavior and preserves
the PrimitiveOS Oasis profile's existing upright base codes, including the
Reader's separate user inversion setting.
[Touch coordinates](https://github.com/koreader/koreader/blob/9192014d8bd82a91dc1012473be0f238dedfdb54/frontend/device/gesturedetector.lua#L1396-L1457) ·
[Touch rotation source](https://github.com/koreader/koreader-base/blob/6e4bc81af4e04f78d81677a323a780a71b29702a/ffi/framebuffer.lua#L120-L126) ·
[Key rotation](https://github.com/koreader/koreader/blob/9192014d8bd82a91dc1012473be0f238dedfdb54/frontend/device/input.lua#L275-L282)

## Smallest shared path

1. Add a four-way logical viewport type alongside the existing portrait
   `Settings::Rotation`. Add an opt-in `Display` entry point that takes a frame
   and explicit logical dimensions, projects it into portrait memory, then
   reuses the existing diff, waveform and update path. Move no existing geometry
   on the first change. Use the firmware configuration read by the draw to validate
   the projected frame before writing. Invalidate the diff baseline on a viewport
   or framebuffer configuration change, even if the frame lengths match. The
   portrait draw currently accepts firmware turns 0 and 2 and rejects mismatched
   width or offsets. A framebuffer that actually reports 1 or 3 needs its own
   measured case before use. [Settings](../crates/kl-core/src/settings.rs) ·
   [Display](../crates/primitive-eink/src/display.rs) ·
   [Panel draw](../crates/primitive-eink/src/lib.rs)

2. Expose opt-in, native input with complete start and end points plus a
   viewport generation. `primitive_input::touch_loop_with_origin` already
   supplies the horizontal origin to the app kit. `kl_kindle::input::touch_loop`
   reduces swipes to `dx`, `dy` and `start_y` for Launcher. That event loses
   the field needed for landscape top-edge detection. Launcher needs complete
   coordinates before it chooses between normal input and an in-app system
   gesture. Discard stale press events when the viewport changes. Feed physical
   button codes through the same viewport selection.
   [Touch decoder](../crates/primitive-input/src/lib.rs) ·
   [Launcher input adapter](../crates/kl-kindle/src/input.rs)

3. Negotiate a new run-scoped viewport record for a landscape-capable Launcher
   and app. Keep the current portrait record path for older builds. The new
   record needs the run owner, generation, logical turn and dimensions, and
   framebuffer configuration. Publish a pending transition before changing
   the displayed frame. Mark it ready only after a successful presentation.
   Launcher can freeze and snapshot a ready, matching generation. Defer an
   overlay while the record is pending or inconsistent. Capturing the fallback
   portrait interpretation at that point could save or restore the wrong pixels.
   Construct the overlay `Layout` and `Renderer` with the app viewport, invert
   its saved framebuffer pixels back to logical coordinates for the backdrop,
   and present through the same transform. Upon exiting the app, Launcher
   resumes its own viewport. The current local Launcher system layer
   (`6028d89`) stores a portrait app turn and raw pixels, and
   `OverlayDisplay` takes an `Option<Rotation>`.
   [Current app turn publisher](../crates/primitive-app/src/orientation.rs)

4. Let Reader opt into preparing, accepting and redrawing a resized viewport.
   Commit view geometry and input generation together. Reflow around the same
   source anchor, keep a fixed document on the same source page, and keep PDF
   document-content rotation separate from the screen turn. Reader's portrait
   preferences and the other apps can keep using the existing API.
   [KOReader reflow on rotation](https://github.com/koreader/koreader/blob/9192014d8bd82a91dc1012473be0f238dedfdb54/frontend/apps/reader/modules/readerview.lua#L892-L919)

## Shared display API

`ViewportTransform` projects and recovers packed eight-bit frames without
including the framebuffer row padding. `Display::present_view` takes a
logical frame, its dimensions and the requested screen turn. It rejects
unsupported panel configurations and clears its diff baseline when the
logical turn or validated framebuffer configuration changes.
`present_view_region` clips in the logical viewport before projecting and
aligning the exact physical update rectangle. The first complete frame after
a region write is redrawn in full.

The existing portrait settings and app kit still use their existing entry
points. This API does not negotiate overlay ownership or resize app layouts.
[Viewport transform](../crates/kl-core/src/viewport.rs) ·
[Viewport presentation](../crates/primitive-eink/src/display.rs)

## Shared input API

Launcher can replace its portrait input loops with
`kl_kindle::input::viewport_touch_loop` and `viewport_button_loop` when
it accepts a four-way viewport. Share one `ViewportGeneration` with those
threads and send each `ViewportEvent` to the Launcher event queue. Maintain
the existing evdev grab at the same boundary when changing overlay focus.

A touch event stores both native points and its press generation, including
every hold stage. Before testing a top edge or dispatching to the visible
layout, call `event.resolve(&generation, visible_turn)` on the event loop.
The result carries `Input` and the full projected touch origin. Framebuffer
turn is not part of this API because it affects raster memory only.

Advance the generation when the event loop commits a new viewport or changes
the visible input owner. Clear any active selection or drag state belonging
to the previous viewport then. Any queued event from the previous generation,
including a release without its original hold start, is discarded. Ignore
input during a pending handoff and resolve new contacts once the visible
viewport is ready. Existing portrait callers can continue using `Ev::Input`.
[Input adapter](../crates/kl-kindle/src/input.rs) ·
[Native viewport events](../crates/primitive-input/src/viewport.rs)

## Scoped viewport handoff

A Launcher that supports v2 can offer `KL_APP_VIEWPORT_FILE` only to eligible
runs. It uses the existing `app-turn-owner` supervisor record as the authority,
with the live PID and its Linux start tick. The per-run viewport path is the
`viewport` sibling of the v1 `turn` file in `app-turn.<token>/`.
Eligibility must come from the supervisor. For an offered run, an absent,
invalid or pending record defers the overlay. The v1 path remains available for
runs without an offer.

The supervisor must identify the executable that actually presents the frame.
An app wrapper should exec that executable so its PID and start tick match.
The app opens `primitive_app::viewport::ViewportPublisher` using the offer.
Its constructor distinguishes an absent offer from a malformed offer.
The app presents through the publisher on every scoped write, including initial
presentation, redraw after resume, and storage transitions. The publisher
atomically stages pending before writing any pixels and commits ready after
the display accepts the frame and checks the same geometry again. An error
can leave the viewport pending even when pixels were written. Retry the same
generation and layout or present a newer generation through the publisher.
The supervisor owns retirement of the run record and directory.

The records are one bounded ASCII line. They name the authorized run even
though the file lives in its scoped directory.

```text
v2 pending PID START SCOPE GENERATION TURN LOGICAL_W LOGICAL_H
v2 ready PID START SCOPE GENERATION TURN LOGICAL_W LOGICAL_H NATIVE_W NATIVE_H F YOFFSET PITCH
```

`TURN` uses `U`, `CW`, `D` or `CCW`. `F` is the measured native framebuffer
turn, currently 0 or 2. Offset is measured in rows and pitch in bytes. A
generation identifies one logical viewport. Repeated frames in that viewport
share its generation. Changes to its logical turn or canvas dimensions must advance it. Follow
and fixed settings stay with the app.

For the consuming Launcher:

1. Obtain current native geometry from `Display::viewport_framebuffer`, then
   call `kl_kindle::viewport::read_ready_viewport` for the active run with a
   minimum generation tracked for that owner. Use the result for preliminary
   hit testing. Defer when any observation is missing.
2. Stop the same foreground app and wait for a stopped process state. Read
   the ready record with `require_stopped` and compare it with the preliminary
   record. If it changed or became pending, resume the app and defer.
3. Capture through `Display::snapshot_viewport(ready.framebuffer)`. It
   verifies the geometry before and after reading packed native rows. Check
   the frozen owner and ready metadata again before accepting the snapshot.
4. Advance Launcher's own input generation when its accepted visible
   viewport or owner changes. Restore exact native pixels only while their
   geometry still matches, or request an app redraw through the publisher.

`primitive_app::run` negotiates once at startup, when Launcher provides a
valid offer and the app returns a viewport request. Without an offer or an
opted-in app, the kit uses the existing portrait lifecycle. The v2 app can
follow the inherited portrait turn or request a fixed logical quarter turn.
The request can change after input or a timer. Returning `None` after
negotiation follows the Launcher while retaining the v2 protocol.

The app implements `viewport_request`, `stage_viewport`,
`draw_staged_viewport`, and `finish_viewport`. Staging keeps the current
layout, frame and saved source state available. The kit creates a canvas
for the proposed dimensions and calls the mutable draw hook for the staged
layout. It sends `Commit` to adopt the candidate after the publisher returns
its verified ready record. `Abort` discards staged resources, including
when staging returned an error. The app may reuse its existing render path
inside the staged draw and restores its accepted state before returning.

A rejected candidate may have reached part of the panel. The kit parks
input before exposure and, when an accepted layout exists, draws that
layout through the publisher with a newer generation. It retries failed
presentations on a bounded timer while awake. The app's accepted layout
stays in memory through recovery. Frame generations identify committed
presentations and are independent of persisted reading positions. Native
contacts received during the handoff, contacts from older generations,
and events whose live framebuffer no longer matches the accepted readback
cannot enter the app. Sleep, Launcher redraw signals, and USB changes
invalidate queued native events. The USB message and pressed control
frames use the same scoped publisher as ordinary app frames.

The host fake-app tests use the shared Noto Sans fonts. Run
`KL_TEST_FONTS=/path/to/NotoSans/fonts cargo test -p primitive-app -- --include-ignored`
to check staging at new dimensions, old and in-flight input rejection,
partial exposure recovery and a live readback mismatch. The Launcher
offer and Reader integration, and physical e-ink acceptance, remain
separate from the host checks.
[Wire parser and observer](../crates/kl-kindle/src/viewport.rs) ·
[Opt-in publisher](../crates/primitive-app/src/viewport.rs)

## Mac proof

Run from the Tools repository root with readable copies of Noto Sans Regular
and Bold, such as the Reader test fonts.

```sh
cargo test --locked -p kl-render --example landscape_portrait_buffer
cargo run --locked -p kl-render --example landscape_portrait_buffer -- \
  path/to/captures path/to/NotoSans-Regular.ttf path/to/NotoSans-Bold.ttf
```

The tests check an asymmetric 3×5 matrix and inverse touch coordinates for all
four turns under firmware turns 0 and 2. The pinned stock Lua
`setRotationMode`, `rotateAbsolute`, `getPhysicalCoordinates`, and
`getPhysicalRect` were run in Lua 5.1 against the same raster matrix.

The example draws an independently authored two-column page with the existing
Canvas. For each landscape turn and firmware turn, it writes into simulated
framebuffer memory with nonzero `yoffset` and sentinel padding. It checks
Quick Settings open from the visible top, rejection of an opposite swipe,
brightness control hit-testing, and dismissal restoring exact original bytes.
It captures the logical reading and overlay frames and the physical portrait
memory. The actual Quick Settings row layout fits 1680×1264.

| Landscape turn | Firmware turn | Aligned overlay update `(x, y, w, h)` in portrait memory |
|---|---|---|
| CW | 0 | (24, 0, 960, 1680) |
| CW | 2 | (280, 0, 960, 1680) |
| CCW | 0 | (280, 0, 960, 1680) |
| CCW | 2 | (24, 0, 960, 1680) |

This is a host proof of geometry, input mapping and memory writes. Physical
update timing, orientation-change races, Reader repagination, full Settings,
lock and USB handoff still require integration and device verification.
