use dioxus::prelude::*;

#[component]
pub fn Header() -> Element {
    rsx! {
        header { class: "header",
            h1 { "⬡ Styrene Mesh" }
            span { class: "subtitle", "Dioxus unified UI spike" }
        }
    }
}
