# UI after gallery (URL-reachable states)

Captured 2026-07-08 from https://alpharomerojl.github.io/reticle/ (served bundle: web-d5e310318aa2611e) by e2e/baseline-gallery.mjs
using Chromium + SwiftShader software GL (the e2e webgl2 flags), device scale 1.

Interior states that need in-app interaction (panels expanded, DRC/diff overlays,
comments, agent panel, 3D stack, cross-section) are captured by the native
demo-script harness in the Wave 5 capture queue, not by this script.

| file | state | size | notes |
|---|---|---|---|
| home-default--1280x800.png | home-default | 1280x800 | landing view exactly as a first visit gets it (replay theater is the public default) |
| home-default--1600x1000.png | home-default | 1600x1000 | landing view exactly as a first visit gets it (replay theater is the public default) |
| home-default--900x600.png | home-default | 900x600 | landing view exactly as a first visit gets it (replay theater is the public default) |
| view-editor--1280x800.png | view-editor | 1280x800 | the editor entry (?view=editor) as it lands, start surface included if shown |
| view-editor--1600x1000.png | view-editor | 1600x1000 | the editor entry (?view=editor) as it lands, start surface included if shown |
| view-editor--900x600.png | view-editor | 900x600 | the editor entry (?view=editor) as it lands, start surface included if shown |
| archive-stream--1280x800.png | archive-stream | 1280x800 | streaming the 3.01 GiB live R2 archive over HTTP Range; streaming HUD active (stat wait timed out on this host; shot after boot + settle) |
| archive-stream--1600x1000.png | archive-stream | 1600x1000 | streaming the 3.01 GiB live R2 archive over HTTP Range; streaming HUD active (stat wait timed out on this host; shot after boot + settle) |
| archive-stream--900x600.png | archive-stream | 900x600 | streaming the 3.01 GiB live R2 archive over HTTP Range; streaming HUD active (stat wait timed out on this host; shot after boot + settle) |
| viewer-empty-room--1280x800.png | viewer-empty-room | 1280x800 | share-link viewer chrome joining a room with no publisher (no live content by design) |
| viewer-empty-room--1600x1000.png | viewer-empty-room | 1600x1000 | share-link viewer chrome joining a room with no publisher (no live content by design) |
| viewer-empty-room--900x600.png | viewer-empty-room | 900x600 | share-link viewer chrome joining a room with no publisher (no live content by design) |
| home-default--phone-412x839.png | home-default | phone 412x839 | landing view exactly as a first visit gets it (replay theater is the public default) |
| view-editor--phone-412x839.png | view-editor | phone 412x839 | the editor entry (?view=editor) as it lands, start surface included if shown |
| archive-stream--phone-412x839.png | archive-stream | phone 412x839 | streaming the 3.01 GiB live R2 archive over HTTP Range; streaming HUD active (stat wait timed out on this host; shot after boot + settle) |
| home-default--tablet-768x1024.png | home-default | tablet 768x1024 | landing view exactly as a first visit gets it (replay theater is the public default) |
| view-editor--tablet-768x1024.png | view-editor | tablet 768x1024 | the editor entry (?view=editor) as it lands, start surface included if shown |
| archive-stream--tablet-768x1024.png | archive-stream | tablet 768x1024 | streaming the 3.01 GiB live R2 archive over HTTP Range; streaming HUD active (stat wait timed out on this host; shot after boot + settle) |
