use std::collections::HashSet;

use knuffel::errors::DecodeError;
use smithay::input::keyboard::Keysym;

use crate::utils::{expect_only_children, MergeWith};
use crate::{Action, Bind, Color, FloatOrInt, Key, Modifiers, Trigger};

#[derive(Debug, PartialEq)]
pub struct RecentWindows {
    pub on: bool,
    pub debounce_ms: u16,
    pub open_delay_ms: u16,
    pub highlight: MruHighlight,
    pub previews: MruPreviews,
    pub binds: Vec<Bind>,
}

impl Default for RecentWindows {
    fn default() -> Self {
        RecentWindows {
            on: true,
            debounce_ms: 750,
            open_delay_ms: 150,
            highlight: MruHighlight::default(),
            previews: MruPreviews::default(),
            binds: default_binds(),
        }
    }
}

#[derive(knuffel::Decode, Debug, Default, PartialEq)]
pub struct RecentWindowsPart {
    #[knuffel(child)]
    pub on: bool,
    #[knuffel(child)]
    pub off: bool,
    #[knuffel(child, unwrap(argument))]
    pub debounce_ms: Option<u16>,
    #[knuffel(child, unwrap(argument))]
    pub open_delay_ms: Option<u16>,
    #[knuffel(child)]
    pub highlight: Option<MruHighlightPart>,
    #[knuffel(child)]
    pub previews: Option<MruPreviewsPart>,
    #[knuffel(child)]
    pub binds: Option<MruBinds>,
}

impl MergeWith<RecentWindowsPart> for RecentWindows {
    fn merge_with(&mut self, part: &RecentWindowsPart) {
        self.on |= part.on;
        if part.off {
            self.on = false;
        }

        merge_clone!((self, part), debounce_ms, open_delay_ms);
        merge!((self, part), highlight, previews);

        if let Some(part) = &part.binds {
            // Remove existing binds matching any new bind.
            self.binds
                .retain(|bind| !part.0.iter().any(|new| new.key == bind.key));
            // Add all new binds.
            self.binds.extend(part.0.iter().cloned().map(Bind::from));
        }
    }
}

#[derive(Debug, PartialEq)]
pub struct MruHighlight {
    pub active_color: Color,
    pub urgent_color: Color,
    pub padding: f64,
    pub corner_radius: f64,
}

impl Default for MruHighlight {
    fn default() -> Self {
        Self {
            active_color: Color::new_unpremul(0.6, 0.6, 0.6, 1.),
            urgent_color: Color::new_unpremul(1., 0.6, 0.6, 1.),
            padding: 30.,
            corner_radius: 0.,
        }
    }
}

#[derive(knuffel::Decode, Debug, Default, PartialEq)]
pub struct MruHighlightPart {
    #[knuffel(child)]
    pub active_color: Option<Color>,
    #[knuffel(child)]
    pub urgent_color: Option<Color>,
    #[knuffel(child, unwrap(argument))]
    pub padding: Option<FloatOrInt<0, 65535>>,
    #[knuffel(child, unwrap(argument))]
    pub corner_radius: Option<FloatOrInt<0, 65535>>,
}

impl MergeWith<MruHighlightPart> for MruHighlight {
    fn merge_with(&mut self, part: &MruHighlightPart) {
        merge_clone!((self, part), active_color, urgent_color);
        merge!((self, part), padding, corner_radius);
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MruPreviews {
    pub max_height: f64,
    pub max_scale: f64,
}

impl Default for MruPreviews {
    fn default() -> Self {
        Self {
            max_height: 480.,
            max_scale: 0.5,
        }
    }
}

#[derive(knuffel::Decode, Debug, Default, PartialEq)]
pub struct MruPreviewsPart {
    #[knuffel(child, unwrap(argument))]
    pub max_height: Option<FloatOrInt<1, 65535>>,
    #[knuffel(child, unwrap(argument))]
    pub max_scale: Option<FloatOrInt<0, 1>>,
}

impl MergeWith<MruPreviewsPart> for MruPreviews {
    fn merge_with(&mut self, part: &MruPreviewsPart) {
        merge!((self, part), max_height, max_scale);
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct MruBind {
    // MRU bind keys must have a modifier, this is enforced during parsing. The switcher will close
    // once all modifiers are released.
    pub key: Key,
    pub action: MruAction,
    pub allow_inhibiting: bool,
    pub hotkey_overlay_title: Option<Option<String>>,
}

impl From<MruBind> for Bind {
    fn from(x: MruBind) -> Self {
        Self {
            key: x.key,
            action: Action::from(x.action),
            repeat: true,
            cooldown: None,
            allow_when_locked: false,
            allow_inhibiting: x.allow_inhibiting,
            hotkey_overlay_title: x.hotkey_overlay_title,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum MruDirection {
    /// Most recently used to least.
    #[default]
    Forward,
    /// Least recently used to most.
    Backward,
}

#[derive(knuffel::DecodeScalar, Clone, Copy, Debug, Default, PartialEq)]
pub enum MruScope {
    /// All windows.
    #[default]
    All,
    /// Windows on the active output.
    Output,
    /// Windows on the active workspace.
    Workspace,
}

#[derive(knuffel::DecodeScalar, Clone, Copy, Debug, Default, PartialEq)]
pub enum MruFilter {
    /// All windows.
    #[default]
    #[knuffel(skip)]
    All,
    /// Windows with the same app id as the active window.
    AppId,
}

#[derive(knuffel::Decode, Debug, Clone, PartialEq)]
pub enum MruAction {
    NextWindow(
        #[knuffel(property(name = "scope"))] Option<MruScope>,
        #[knuffel(property(name = "filter"), default)] MruFilter,
    ),
    PreviousWindow(
        #[knuffel(property(name = "scope"))] Option<MruScope>,
        #[knuffel(property(name = "filter"), default)] MruFilter,
    ),
}

impl From<MruAction> for Action {
    fn from(x: MruAction) -> Self {
        match x {
            MruAction::NextWindow(scope, filter) => Self::MruAdvance {
                direction: MruDirection::Forward,
                scope,
                filter: Some(filter),
            },
            MruAction::PreviousWindow(scope, filter) => Self::MruAdvance {
                direction: MruDirection::Backward,
                scope,
                filter: Some(filter),
            },
        }
    }
}

#[derive(Debug, Default, PartialEq)]
pub struct MruBinds(pub Vec<MruBind>);

fn default_binds() -> Vec<Bind> {
    let mut rv = Vec::new();

    let mut push = |trigger, base_mod, filter| {
        rv.push(Bind::from(MruBind {
            key: Key {
                trigger: Trigger::Keysym(trigger),
                modifiers: base_mod,
            },
            action: MruAction::NextWindow(None, filter),
            allow_inhibiting: true,
            hotkey_overlay_title: None,
        }));
        rv.push(Bind::from(MruBind {
            key: Key {
                trigger: Trigger::Keysym(trigger),
                modifiers: base_mod | Modifiers::SHIFT,
            },
            action: MruAction::PreviousWindow(None, filter),
            allow_inhibiting: true,
            hotkey_overlay_title: None,
        }));
    };

    for base_mod in [Modifiers::ALT, Modifiers::COMPOSITOR] {
        push(Keysym::Tab, base_mod, MruFilter::All);
        push(Keysym::grave, base_mod, MruFilter::AppId);
    }

    rv
}

impl<S> knuffel::Decode<S> for MruBinds
where
    S: knuffel::traits::ErrorSpan,
{
    fn decode_node(
        node: &knuffel::ast::SpannedNode<S>,
        ctx: &mut knuffel::decode::Context<S>,
    ) -> Result<Self, DecodeError<S>> {
        expect_only_children(node, ctx);

        let mut seen_keys = HashSet::new();

        let mut binds = Vec::new();

        for child in node.children() {
            match MruBind::decode_node(child, ctx) {
                Ok(bind) => {
                    if !seen_keys.insert(bind.key) {
                        ctx.emit_error(DecodeError::unexpected(
                            &child.node_name,
                            "keybind",
                            "duplicate keybind",
                        ));
                        continue;
                    }

                    binds.push(bind);
                }
                Err(e) => {
                    ctx.emit_error(e);
                }
            }
        }

        Ok(Self(binds))
    }
}

impl<S> knuffel::Decode<S> for MruBind
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

        let key = node
            .node_name
            .parse::<Key>()
            .map_err(|e| DecodeError::conversion(&node.node_name, e.wrap_err("invalid keybind")))?;

        // A modifier is required because MRU remains on screen as long as any modifier is held.
        if key.modifiers.is_empty() {
            ctx.emit_error(DecodeError::unexpected(
                &node.node_name,
                "keybind",
                "keybind must have a modifier key",
            ));
        }

        // FIXME: To support this, all the mods_with_mouse_binds()/mods_with_wheel_binds()/etc.
        // will need to learn about recent-windows bindings.
        if !matches!(key.trigger, Trigger::Keysym(_)) {
            ctx.emit_error(DecodeError::unexpected(
                &node.node_name,
                "key",
                "key must be a keyboard key (others are unsupported here for now)",
            ));
        }

        let mut allow_inhibiting = true;
        let mut hotkey_overlay_title = None;
        for (name, val) in &node.properties {
            match &***name {
                "allow-inhibiting" => {
                    allow_inhibiting = knuffel::traits::DecodeScalar::decode(val, ctx)?;
                }
                "hotkey-overlay-title" => {
                    hotkey_overlay_title = Some(knuffel::traits::DecodeScalar::decode(val, ctx)?);
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

        let mut children = node.children();

        // If the action is invalid but the key is fine, we still want to return something.
        // That way, the parent can handle the existence of duplicate keybinds,
        // even if their contents are not valid.
        let dummy = Self {
            key,
            action: MruAction::NextWindow(None, MruFilter::All),
            allow_inhibiting: true,
            hotkey_overlay_title: None,
        };

        if let Some(child) = children.next() {
            for unwanted_child in children {
                ctx.emit_error(DecodeError::unexpected(
                    unwanted_child,
                    "node",
                    "only one action is allowed per keybind",
                ));
            }
            match MruAction::decode_node(child, ctx) {
                Ok(action) => Ok(Self {
                    key,
                    action,
                    allow_inhibiting,
                    hotkey_overlay_title,
                }),
                Err(e) => {
                    ctx.emit_error(e);
                    Ok(dummy)
                }
            }
        } else {
            ctx.emit_error(DecodeError::missing(
                node,
                "expected an action for this keybind",
            ));
            Ok(dummy)
        }
    }
}
