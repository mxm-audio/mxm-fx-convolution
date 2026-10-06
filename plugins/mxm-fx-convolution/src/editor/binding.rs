//! Binding one parameter to one `mxm-ui` control: **the collection's one binding**,
//! `mxm_preset::binding` (`plans/plan-editor-standard.md` R1c). Every editor carried its own copy
//! of it until 2026-09-24; this module re-exports the one, so `super::binding::…` paths keep
//! meaning what they did.

pub use mxm_preset::binding::*;
