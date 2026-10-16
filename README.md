# mxm-fx-convolution

An MXM effect: A WAV-based convolution effect with reversible response preparation.

Part of the MXM collection: every instrument, effect and tool lives in its own repository under
[github.com/mxm-audio](https://github.com/mxm-audio), built on
[mxm-kit](https://github.com/mxm-audio/mxm-kit).

## Building

Rust 1.95 or newer. On Linux, install ALSA, JACK, X11, xkbcommon and a GL loader first.

```bash
cargo xtask bundle mxm-fx-convolution --release   # -> target/bundled/mxm-fx-convolution.clap and .vst3
cargo test
```

The bundle installs itself, CLAP and VST3, in the standard plugin folders: on Windows
`C:\Program Files\Common Files\CLAP` and `…\VST3` (give your account write access to them once),
on macOS `~/Library/Audio/Plug-Ins/CLAP` and `…/VST3`, on Linux `~/.clap` and `~/.vst3`.
`MXM_CLAP_DIR` and `MXM_VST3_DIR` choose other folders, and `MXM_INSTALL_CLAP=0` turns installing
off. This is pre-alpha: nothing is released, so there are no official builds yet.

## Licence

GPL-3.0-or-later — see [`LICENSE`](LICENSE) and [`NOTICE.md`](NOTICE.md). The MXM
name and logo are trademarks; [`TRADEMARKS.md`](TRADEMARKS.md) says how they may be used.

Contributions are welcome: see [`CONTRIBUTING.md`](CONTRIBUTING.md). How the repository is
organised, and the rules each part keeps, are in [`AGENTS.md`](AGENTS.md).
