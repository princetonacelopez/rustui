//! AccessKit tree builder and action dispatcher bridging `rustui-core` nodes to AT-SPI,
//! NSAccessibility, and UI Automation.

use accesskit::{
    Action, ActionData, ActionRequest, Live, Node as AkNode, NodeId as AkNodeId, Rect as AkRect,
    Role, TreeId, TreeInfo, TreeUpdate,
};
use rustui_core::{AccessibilityLive, AccessibilityRole, NodeId, NodeKind, Runtime, RuntimeInner};
use slotmap::{Key, KeyData};

use crate::input::FocusManager;

/// Converts between `rustui_core::NodeId` and `accesskit::NodeId`.
#[inline]
pub fn to_ak_id(id: NodeId) -> AkNodeId {
    AkNodeId(id.data().as_ffi())
}

/// Converts an `accesskit::NodeId` back to `rustui_core::NodeId`.
#[inline]
pub fn from_ak_id(id: AkNodeId) -> NodeId {
    NodeId::from(KeyData::from_ffi(id.0))
}

/// Synthetic root AccessKit node ID representing the OS window container.
pub const WINDOW_AK_ROOT_ID: AkNodeId = AkNodeId(u64::MAX - 1);

/// Bridges retained UI nodes and AccessKit accessibility trees & actions.
#[derive(Debug, Default)]
pub struct AccessKitBridge;

impl AccessKitBridge {
    /// Builds a complete [`TreeUpdate`] for the window rooted at `root`.
    pub fn build_tree_update(
        runtime: &Runtime,
        root: NodeId,
        window_title: &str,
        focused: Option<NodeId>,
    ) -> TreeUpdate {
        let mut nodes = Vec::new();
        runtime.with_inner(|inner| {
            collect_ak_nodes(inner, root, &mut nodes);
        });

        let mut win_node = AkNode::new(Role::Window);
        win_node.set_label(window_title);
        if runtime.with_inner(|inner| {
            inner
                .nodes
                .get(root)
                .is_some_and(|n| !n.accessibility.hidden && !n.visual_props.hidden)
        }) {
            win_node.set_children(vec![to_ak_id(root)]);
        }
        nodes.push((WINDOW_AK_ROOT_ID, win_node));

        let focus_id = focused
            .filter(|id| {
                runtime.with_inner(|inner| {
                    inner
                        .nodes
                        .get(*id)
                        .is_some_and(|n| !n.accessibility.hidden && !n.visual_props.hidden)
                })
            })
            .map(to_ak_id)
            .unwrap_or(WINDOW_AK_ROOT_ID);

        TreeUpdate {
            nodes,
            tree: Some(TreeInfo::new(WINDOW_AK_ROOT_ID)),
            tree_id: TreeId::ROOT,
            focus: focus_id,
        }
    }

    /// Dispatches an [`ActionRequest`] from a screen reader or AT tool into UI handlers.
    pub fn handle_action_request(
        runtime: &Runtime,
        focus: &mut FocusManager,
        req: &ActionRequest,
    ) -> bool {
        if req.target_node == WINDOW_AK_ROOT_ID {
            return false;
        }
        let node_id = from_ak_id(req.target_node);
        let exists = runtime.with_inner(|inner| inner.nodes.contains_key(node_id));
        if !exists {
            return false;
        }

        match req.action {
            Action::Focus => {
                focus.set_focus(runtime, Some(node_id));
                true
            }
            Action::Click => {
                let cb = runtime.with_inner(|inner| {
                    inner
                        .nodes
                        .get(node_id)
                        .and_then(|n| n.events.on_tap.clone())
                });
                if let Some(f) = cb {
                    runtime.enter(|| f());
                    return true;
                }
                false
            }
            Action::SetValue => {
                let val_str = match &req.data {
                    Some(ActionData::Value(v)) => Some(v.to_string()),
                    Some(ActionData::NumericValue(n)) => Some(n.to_string()),
                    _ => None,
                };
                if let Some(s) = val_str {
                    let cb = runtime.with_inner(|inner| {
                        inner
                            .nodes
                            .get(node_id)
                            .and_then(|n| n.events.on_accessibility_set_value.clone())
                    });
                    if let Some(f) = cb {
                        runtime.enter(|| f(s));
                        return true;
                    }
                }
                false
            }
            Action::Increment => {
                let cb = runtime.with_inner(|inner| {
                    inner
                        .nodes
                        .get(node_id)
                        .and_then(|n| n.events.on_accessibility_increment.clone())
                });
                if let Some(f) = cb {
                    runtime.enter(|| f());
                    return true;
                }
                false
            }
            Action::Decrement => {
                let cb = runtime.with_inner(|inner| {
                    inner
                        .nodes
                        .get(node_id)
                        .and_then(|n| n.events.on_accessibility_decrement.clone())
                });
                if let Some(f) = cb {
                    runtime.enter(|| f());
                    return true;
                }
                false
            }
            Action::CustomAction => {
                if let Some(ActionData::CustomAction(action_idx)) = req.data {
                    let cb = runtime.with_inner(|inner| {
                        let node = inner.nodes.get(node_id)?;
                        let name = node
                            .accessibility
                            .custom_action_names
                            .get(action_idx as usize)?;
                        node.events.custom_actions.get(name).cloned()
                    });
                    if let Some(f) = cb {
                        runtime.enter(|| f());
                        return true;
                    }
                }
                false
            }
            _ => false,
        }
    }
}

fn collect_combined_text(inner: &RuntimeInner, node_id: NodeId, parts: &mut Vec<String>) {
    let Some(node) = inner.nodes.get(node_id) else {
        return;
    };
    if node.visual_props.hidden || node.accessibility.hidden {
        return;
    }
    if let Some(lbl) = &node.accessibility.label {
        if !lbl.is_empty() {
            parts.push(lbl.clone());
        }
    } else if let NodeKind::Text(t) = &node.kind
        && !t.text.is_empty()
    {
        parts.push(t.text.clone());
    }
    for &cid in &node.children {
        collect_combined_text(inner, cid, parts);
    }
}

fn collect_ak_nodes(inner: &RuntimeInner, node_id: NodeId, out: &mut Vec<(AkNodeId, AkNode)>) {
    let Some(node) = inner.nodes.get(node_id) else {
        return;
    };
    if node.visual_props.hidden || node.accessibility.hidden {
        return;
    }

    let role = map_role(node.accessibility.role, &node.kind);
    let mut ak = AkNode::new(role);

    if let Some(label) = &node.accessibility.label {
        ak.set_label(label.as_str());
    } else if node.accessibility.combine_children {
        let mut combined = Vec::new();
        for &cid in &node.children {
            collect_combined_text(inner, cid, &mut combined);
        }
        if !combined.is_empty() {
            ak.set_label(combined.join(" "));
        }
    } else if let NodeKind::Text(t) = &node.kind
        && !t.text.is_empty()
    {
        ak.set_label(t.text.as_str());
    }

    if let Some(hint) = &node.accessibility.hint
        && !hint.is_empty()
    {
        ak.set_description(hint.as_str());
    }

    match node.accessibility.live {
        AccessibilityLive::Off => {}
        AccessibilityLive::Polite => ak.set_live(Live::Polite),
        AccessibilityLive::Assertive => ak.set_live(Live::Assertive),
    }

    if let Some(val) = &node.accessibility.value {
        ak.set_value(val.as_str());
    } else if let NodeKind::Control(ctrl) = &node.kind {
        if let Some(b) = ctrl.value_bool {
            ak.set_value(if b { "true" } else { "false" });
        } else if let Some(v) = ctrl.value_f32 {
            ak.set_numeric_value(f64::from(v));
        } else if let Some(s) = &ctrl.value_text {
            ak.set_value(s.as_str());
        }
    }

    let b = node.bounds();
    if b.width > 0.0 || b.height > 0.0 {
        ak.set_bounds(AkRect {
            x0: f64::from(b.x),
            y0: f64::from(b.y),
            x1: f64::from(b.max_x()),
            y1: f64::from(b.max_y()),
        });
    }

    if node.accessibility.supports_click || node.events.on_tap.is_some() {
        ak.add_action(Action::Click);
    }
    if node.accessibility.supports_focus || node.focus.focusable {
        ak.add_action(Action::Focus);
    }
    if node.accessibility.supports_set_value || node.events.on_accessibility_set_value.is_some() {
        ak.add_action(Action::SetValue);
    }
    if node.accessibility.supports_increment || node.events.on_accessibility_increment.is_some() {
        ak.add_action(Action::Increment);
    }
    if node.accessibility.supports_decrement || node.events.on_accessibility_decrement.is_some() {
        ak.add_action(Action::Decrement);
    }
    if !node.accessibility.custom_action_names.is_empty() {
        ak.add_action(Action::CustomAction);
        for (idx, name) in node.accessibility.custom_action_names.iter().enumerate() {
            ak.push_custom_action(accesskit::CustomAction {
                id: idx as i32,
                description: name.clone(),
            });
        }
    }

    if !node.accessibility.combine_children {
        let mut visible_children: Vec<NodeId> = node
            .children
            .iter()
            .copied()
            .filter(|cid| {
                inner
                    .nodes
                    .get(*cid)
                    .is_some_and(|c| !c.visual_props.hidden && !c.accessibility.hidden)
            })
            .collect();
        visible_children.sort_by(|a, b| {
            let na = &inner.nodes[*a];
            let nb = &inner.nodes[*b];
            nb.accessibility
                .sort_priority
                .total_cmp(&na.accessibility.sort_priority)
        });

        let mut ak_children = Vec::with_capacity(visible_children.len());
        for cid in visible_children {
            ak_children.push(to_ak_id(cid));
            collect_ak_nodes(inner, cid, out);
        }
        if !ak_children.is_empty() {
            ak.set_children(ak_children);
        }
    }

    out.push((to_ak_id(node_id), ak));
}

fn map_role(role: AccessibilityRole, kind: &NodeKind) -> Role {
    if role != AccessibilityRole::GenericContainer {
        return match role {
            AccessibilityRole::GenericContainer => Role::GenericContainer,
            AccessibilityRole::Window => Role::Window,
            AccessibilityRole::Group => Role::Group,
            AccessibilityRole::Heading => Role::Heading,
            AccessibilityRole::Label => Role::Label,
            AccessibilityRole::Button => Role::Button,
            AccessibilityRole::CheckBox => Role::CheckBox,
            AccessibilityRole::Switch => Role::Switch,
            AccessibilityRole::Slider => Role::Slider,
            AccessibilityRole::ComboBox => Role::ComboBox,
            AccessibilityRole::TextInput => Role::TextInput,
            AccessibilityRole::MultilineTextInput => Role::MultilineTextInput,
            AccessibilityRole::PasswordInput => Role::PasswordInput,
            AccessibilityRole::Image => Role::Image,
            AccessibilityRole::Link => Role::Link,
            AccessibilityRole::List => Role::List,
            AccessibilityRole::ListItem => Role::ListItem,
            AccessibilityRole::Table => Role::Table,
            AccessibilityRole::TableRow => Role::Row,
            AccessibilityRole::TableColumnHeader => Role::ColumnHeader,
            AccessibilityRole::TableCell => Role::Cell,
            AccessibilityRole::Tree => Role::Tree,
            AccessibilityRole::TreeItem => Role::TreeItem,
            AccessibilityRole::TabList => Role::TabList,
            AccessibilityRole::Tab => Role::Tab,
            AccessibilityRole::Toolbar => Role::Toolbar,
            AccessibilityRole::Menu => Role::Menu,
            AccessibilityRole::MenuBar => Role::MenuBar,
            AccessibilityRole::MenuItem => Role::MenuItem,
            AccessibilityRole::Popover => Role::Dialog,
            AccessibilityRole::ScrollView => Role::ScrollView,
            AccessibilityRole::Splitter => Role::Splitter,
            AccessibilityRole::Navigation => Role::Navigation,
            AccessibilityRole::ProgressIndicator => Role::ProgressIndicator,
            AccessibilityRole::Meter => Role::Meter,
            AccessibilityRole::AlertDialog => Role::AlertDialog,
            AccessibilityRole::Status => Role::Status,
        };
    }

    match kind {
        NodeKind::Text(_) => Role::Label,
        NodeKind::Image(_) => Role::Image,
        NodeKind::Scroll(_) => Role::ScrollView,
        NodeKind::VirtualList(_) => Role::List,
        NodeKind::Control(c) => match c.kind {
            "button" => Role::Button,
            "toggle" => Role::Switch,
            "checkbox" => Role::CheckBox,
            "slider" => Role::Slider,
            "picker" => Role::ComboBox,
            "textfield" => Role::TextInput,
            "texteditor" | "editorsurface" => Role::MultilineTextInput,
            "securefield" => Role::PasswordInput,
            "progressview" => Role::ProgressIndicator,
            "gauge" => Role::Meter,
            _ => Role::GenericContainer,
        },
        _ => Role::GenericContainer,
    }
}
