//! Styrene DX — Dioxus cross-platform UI spike
//!
//! This spike validates:
//! 1. Protocol crates (styrene-rns identity/crypto) compile into a Dioxus app
//! 2. Shared component model renders on desktop (and eventually web/TUI)
//! 3. Architecture for reactive mesh status display
//!
//! Run: `dx serve` (web) or `cargo run` (desktop)

use dioxus::prelude::*;

mod components;
mod state;

fn main() {
    launch(App);
}

#[component]
fn App() -> Element {
    // Generate a fresh RNS identity to prove crypto crates compile & work
    let identity_info = use_signal(state::generate_identity_info);

    rsx! {
        document::Stylesheet { href: asset!("src/assets/style.css") }

        div { class: "app",
            components::Header {}

            div { class: "content",
                components::IdentityCard { info: identity_info() }
                components::MeshStatus {}
            }
        }
    }
}
