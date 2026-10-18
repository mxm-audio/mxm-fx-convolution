# Notice

mxm-fx-convolution — Copyright (C) 2026 mxm. Free software under the GNU General Public License,
version 3 or later: see `LICENSE`. It comes with ABSOLUTELY NO WARRANTY. The MXM name and logo
are not covered by that licence; see `TRADEMARKS.md`.

## Third-party code in its builds

- [mxm-kit](https://github.com/mxm-audio/mxm-kit) — MIT.
- nice-plug ([mxm-audio/nice-plug](https://github.com/mxm-audio/nice-plug)) — ISC, Copyright (c)
  2022-2024 Robbert van der Helm.
- egui, egui-baseview and their dependencies — MIT or Apache-2.0.
- The VST3 bindings, the `vst3` crate — MIT or Apache-2.0, generated from Steinberg's VST 3 SDK
  3.8.0, which is MIT: Copyright (c) 2025, Steinberg Media Technologies GmbH, its notice below.
- The Inter typeface, embedded by `mxm-ui` — SIL Open Font License 1.1.
- symphonia, through `mxm-audio-file-decode` — MPL-2.0, used unmodified; source at
  https://github.com/pdeljanov/Symphonia.

Every dependency and its licence is in `Cargo.lock`; a release lists them in full.

VST is a registered trademark of Steinberg Media Technologies GmbH.

## Steinberg's VST 3 SDK licence

```text
MIT License

Copyright (c) 2025, Steinberg Media Technologies GmbH

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```
