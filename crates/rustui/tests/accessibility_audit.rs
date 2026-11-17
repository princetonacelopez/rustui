//! Accessibility audit & keyboard-only traversal tests across built-in widgets (Section 8 & 10).

use rustui::platform::to_ak_id;
use rustui::prelude::*;

#[test]
fn accesskit_tree_exposes_roles_labels_actions_and_supports_keyboard_traversal() {
    struct AccessibleApp;
    impl View for AccessibleApp {
        fn body(self, cx: &mut Cx) -> impl View {
            let toggle = cx.state(false);
            let checked = cx.state(false);
            let slider = cx.state(2.0f32);
            let picker = cx.state(0usize);
            let text = cx.state(String::new());
            let secret = cx.state(String::new());
            let clicks = cx.state(0u32);

            VStack::new(
                8,
                (
                    Toolbar::new(Text::new("Accessible App")),
                    Button::new("Primary Action", move || clicks.update(|c| *c += 1)),
                    Toggle::new("Notifications", toggle.binding()),
                    Checkbox::new("Remember Me", checked.binding()),
                    Slider::new(slider.binding(), 0.0, 10.0).step(1.0),
                    Picker::new(picker.binding(), vec!["One".into(), "Two".into()]),
                    TextField::new("Username", text.binding()),
                    SecureField::new("Password", secret.binding()),
                    Text::new("Secret Hidden").accessibility_hidden(true),
                    Text::new(move || format!("clicks={}", clicks.get()))
                        .accessibility_label("Click Counter"),
                ),
            )
            .padding(12)
        }
    }

    let mut sim = HeadlessSimulator::new(AccessibleApp, Size::new(600.0, 700.0));
    let tree = sim.accessibility_tree();

    assert!(
        tree.nodes
            .iter()
            .any(|(_, n)| format!("{:?}", n.role()) == "Window")
    );
    assert!(
        tree.nodes
            .iter()
            .any(|(_, n)| format!("{:?}", n.role()) == "Toolbar")
    );
    assert!(
        tree.nodes
            .iter()
            .any(|(_, n)| format!("{:?}", n.role()) == "Button"
                && n.label() == Some("Primary Action"))
    );
    assert!(
        tree.nodes
            .iter()
            .any(|(_, n)| format!("{:?}", n.role()) == "Switch"
                && n.label() == Some("Notifications"))
    );
    assert!(
        tree.nodes
            .iter()
            .any(|(_, n)| format!("{:?}", n.role()) == "CheckBox"
                && n.label() == Some("Remember Me"))
    );
    assert!(
        tree.nodes
            .iter()
            .any(|(_, n)| format!("{:?}", n.role()) == "Slider")
    );
    assert!(
        tree.nodes
            .iter()
            .any(|(_, n)| format!("{:?}", n.role()) == "ComboBox")
    );
    assert!(tree.nodes.iter().any(|(_, n)| format!("{:?}", n.role()) == "TextInput" && n.label() == Some("Username")));
    assert!(tree.nodes.iter().any(
        |(_, n)| format!("{:?}", n.role()) == "PasswordInput" && n.label() == Some("Password")
    ));
    assert!(
        !tree
            .nodes
            .iter()
            .any(|(_, n)| n.label() == Some("Secret Hidden")),
        "accessibility_hidden(true) node must be excluded from AccessKit tree"
    );

    // Traverse all focusable controls via Tab and verify focus updates in AccessKit tree.
    let first = sim.focus_next().expect("focusable first node");
    let tree_after_focus = sim.accessibility_tree();
    assert_eq!(tree_after_focus.focus, to_ak_id(first));

    // Activate focused Primary Action button with Enter.
    sim.press_key(KeyCode::Enter, Modifiers::NONE);
    assert!(sim.find_node_by_label("clicks=1").is_some());
}
