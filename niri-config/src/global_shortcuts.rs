use std::collections::HashSet;

use knuffel::errors::DecodeError;

use crate::binds::Key;
use crate::utils::RegexEq;

#[derive(Debug, Default, PartialEq)]
pub struct GlobalShortcuts(pub Vec<GlobalShortcut>);

#[derive(Debug, Clone, PartialEq)]
pub struct GlobalShortcut {
    pub trigger: Key,
    pub intercept: bool,
    pub app_id: Selector,
    pub shortcut_id: Selector,
}

#[derive(knuffel::Decode, Debug, Clone, PartialEq)]
pub enum Selector {
    Exact(#[knuffel(argument)] String),
    Match(#[knuffel(argument, str)] RegexEq),
    NeverMatch,
}
impl Selector {
    /// Compares the selector against a `str`
    pub fn matches<T: AsRef<str>>(&self, v: T) -> bool {
        match self {
            Selector::Exact(pred) => pred == v.as_ref(),
            Selector::Match(regex_eq) => regex_eq.0.is_match(v.as_ref()),
            Selector::NeverMatch => false,
        }
    }
}

impl<S> knuffel::Decode<S> for GlobalShortcuts
where
    S: knuffel::traits::ErrorSpan,
{
    fn decode_node(
        node: &knuffel::ast::SpannedNode<S>,
        ctx: &mut knuffel::decode::Context<S>,
    ) -> Result<Self, DecodeError<S>> {
        let mut seen_keys = HashSet::new();
        let mut shortcuts = Vec::new();

        for child in node.children() {
            match GlobalShortcut::decode_node(child, ctx) {
                Err(e) => ctx.emit_error(e),
                Ok(shortcut) => {
                    if seen_keys.insert(shortcut.trigger) {
                        shortcuts.push(shortcut);
                    } else {
                        // This suffers from the same issue mentioned in the `Binds` Decode impl
                        ctx.emit_error(DecodeError::unexpected(
                            &child.node_name,
                            "keybind",
                            "duplicate keybind",
                        ));
                    }
                }
            };
        }

        Ok(Self(shortcuts))
    }
}

impl<S> knuffel::Decode<S> for GlobalShortcut
where
    S: knuffel::traits::ErrorSpan,
{
    fn decode_node(
        node: &knuffel::ast::SpannedNode<S>,
        ctx: &mut knuffel::decode::Context<S>,
    ) -> Result<Self, DecodeError<S>> {
        if let Some(type_name) = &node.type_name {
            ctx.emit_error(DecodeError::unexpected(
                type_name,
                "type name",
                "no type name expected for this node",
            ));
        }

        for val in node.arguments.iter() {
            ctx.emit_error(DecodeError::unexpected(
                &val.literal,
                "argument",
                "no arguments expected for this node",
            ));
        }

        let mut intercept = true;
        for (name, val) in &node.properties {
            match &***name {
                "intercept" => {
                    intercept = knuffel::traits::DecodeScalar::decode(val, ctx)?;
                }
                name_str => {
                    ctx.emit_error(DecodeError::unexpected(
                        name,
                        "property",
                        format!("unexpected property `{}`", name_str.escape_default()),
                    ));
                }
            }
        }

        let key = node
            .node_name
            .parse::<Key>()
            .map_err(|e| DecodeError::conversion(&node.node_name, e.wrap_err("invalid keybind")))?;

        let mut app_id = None;
        let mut shortcut_id = None;
        for child in node.children() {
            match &**child.node_name {
                "app-id" => app_id = decode_selector_child(child, ctx),
                "shortcut-id" => shortcut_id = decode_selector_child(child, ctx),
                name_str => {
                    ctx.emit_error(DecodeError::unexpected(
                        child,
                        "node",
                        format!("unexpected child `{}`", name_str.escape_default()),
                    ));
                }
            }
        }

        let app_id = app_id.unwrap_or(Selector::NeverMatch);
        let shortcut_id = shortcut_id.unwrap_or(Selector::NeverMatch);
        Ok(Self {
            trigger: key,
            intercept,
            app_id,
            shortcut_id,
        })
    }
}

fn decode_selector_child<S: knuffel::traits::ErrorSpan>(
    child: &knuffel::ast::SpannedNode<S>,
    ctx: &mut knuffel::decode::Context<S>,
) -> Option<Selector> {
    let mut grand_children = child.children();
    if let Some(grand_child) = grand_children.next() {
        for unwanted_child in grand_children {
            ctx.emit_error(DecodeError::unexpected(
                unwanted_child,
                "node",
                "only one selector is allowed per attribute",
            ));
        }
        match <Selector as knuffel::Decode<S>>::decode_node(grand_child, ctx) {
            Ok(v) => Some(v),
            Err(e) => {
                ctx.emit_error(e);
                None
            }
        }
    } else {
        ctx.emit_error(DecodeError::missing(
            child,
            "expected a selector for this field",
        ));
        None
    }
}
