use dioxus::prelude::*;

use crate::state::IdentityInfo;

#[component]
pub fn IdentityCard(info: IdentityInfo) -> Element {
    rsx! {
        div { class: "card",
            h2 { "Local Identity" }
            div { class: "field",
                label { "Hash" }
                code { "{info.hash_hex}" }
            }
            div { class: "field",
                label { "Public Key" }
                code { class: "truncate", "{info.public_key_hex}" }
            }
            div { class: "field",
                label { "Signing Key" }
                code { class: "truncate", "{info.signing_key_hex}" }
            }
        }
    }
}
