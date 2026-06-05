use knus::errors::DecodeError;
use niri_ipc::{ColumnDisplay, SizeChange};

use crate::appearance::{
    Border, DEFAULT_BACKGROUND_COLOR, FocusRing, InsertHint, Shadow, TabIndicator,
};
use crate::utils::{Flag, MergeWith, expect_only_children};
use crate::{BorderRule, Color, FloatOrInt, InsertHintPart, ShadowRule, TabIndicatorPart};

#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    pub focus_ring: FocusRing,
    pub border: Border,
    pub shadow: Shadow,
    pub tab_indicator: TabIndicator,
    pub insert_hint: InsertHint,
    pub preset_column_widths: Vec<PresetSize>,
    pub default_column_width: Option<PresetSize>,
    pub preset_window_heights: Vec<PresetSize>,
    pub center_focused_column: CenterFocusedColumn,
    pub always_center_single_column: bool,
    pub empty_workspace_above_first: bool,
    pub default_column_display: ColumnDisplay,
    pub gaps: f64,
    pub struts: Struts,
    pub background_color: Color,
}

impl Default for Layout {
    fn default() -> Self {
        Self {
            focus_ring: FocusRing::default(),
            border: Border::default(),
            shadow: Shadow::default(),
            tab_indicator: TabIndicator::default(),
            insert_hint: InsertHint::default(),
            preset_column_widths: vec![
                PresetSize::Proportion(1. / 3.),
                PresetSize::Proportion(0.5),
                PresetSize::Proportion(2. / 3.),
            ],
            default_column_width: Some(PresetSize::Proportion(0.5)),
            center_focused_column: CenterFocusedColumn::Never,
            always_center_single_column: false,
            empty_workspace_above_first: false,
            default_column_display: ColumnDisplay::Normal,
            gaps: 16.,
            struts: Struts::default(),
            preset_window_heights: vec![
                PresetSize::Proportion(1. / 3.),
                PresetSize::Proportion(0.5),
                PresetSize::Proportion(2. / 3.),
            ],
            background_color: DEFAULT_BACKGROUND_COLOR,
        }
    }
}

impl MergeWith<LayoutPart> for Layout {
    fn merge_with(&mut self, part: &LayoutPart) {
        merge!(
            (self, part),
            focus_ring,
            border,
            shadow,
            tab_indicator,
            insert_hint,
            always_center_single_column,
            empty_workspace_above_first,
            gaps,
        );

        merge_clone!(
            (self, part),
            preset_column_widths,
            preset_window_heights,
            center_focused_column,
            default_column_display,
            struts,
            background_color,
        );

        if let Some(x) = part.default_column_width {
            self.default_column_width = x.0;
        }

        if self.preset_column_widths.is_empty() {
            self.preset_column_widths = Layout::default().preset_column_widths;
        }

        if self.preset_window_heights.is_empty() {
            self.preset_window_heights = Layout::default().preset_window_heights;
        }
    }
}

#[derive(knus::Decode, Debug, Default, Clone, PartialEq)]
pub struct LayoutPart {
    #[knus(child)]
    pub focus_ring: Option<BorderRule>,
    #[knus(child)]
    pub border: Option<BorderRule>,
    #[knus(child)]
    pub shadow: Option<ShadowRule>,
    #[knus(child)]
    pub tab_indicator: Option<TabIndicatorPart>,
    #[knus(child)]
    pub insert_hint: Option<InsertHintPart>,
    #[knus(child, unwrap(children))]
    pub preset_column_widths: Option<Vec<PresetSize>>,
    #[knus(child)]
    pub default_column_width: Option<DefaultPresetSize>,
    #[knus(child, unwrap(children))]
    pub preset_window_heights: Option<Vec<PresetSize>>,
    #[knus(child, unwrap(argument))]
    pub center_focused_column: Option<CenterFocusedColumn>,
    #[knus(child)]
    pub always_center_single_column: Option<Flag>,
    #[knus(child)]
    pub empty_workspace_above_first: Option<Flag>,
    #[knus(child, unwrap(argument, str))]
    pub default_column_display: Option<ColumnDisplay>,
    #[knus(child, unwrap(argument))]
    pub gaps: Option<FloatOrInt<0, 65535>>,
    #[knus(child)]
    pub struts: Option<Struts>,
    #[knus(child)]
    pub background_color: Option<Color>,
}

#[derive(knus::Decode, Debug, Clone, Copy, PartialEq)]
pub enum PresetSize {
    Proportion(#[knus(argument)] f64),
    Fixed(#[knus(argument)] i32),
}

impl From<PresetSize> for SizeChange {
    fn from(value: PresetSize) -> Self {
        match value {
            PresetSize::Proportion(prop) => SizeChange::SetProportion(prop * 100.),
            PresetSize::Fixed(fixed) => SizeChange::SetFixed(fixed),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DefaultPresetSize(pub Option<PresetSize>);

#[derive(knus::Decode, Debug, Default, Clone, Copy, PartialEq)]
pub struct Struts {
    #[knus(child, unwrap(argument), default)]
    pub left: FloatOrInt<-65535, 65535>,
    #[knus(child, unwrap(argument), default)]
    pub right: FloatOrInt<-65535, 65535>,
    #[knus(child, unwrap(argument), default)]
    pub top: FloatOrInt<-65535, 65535>,
    #[knus(child, unwrap(argument), default)]
    pub bottom: FloatOrInt<-65535, 65535>,
}

#[derive(knus::DecodeScalar, Debug, Default, PartialEq, Eq, Clone, Copy)]
pub enum CenterFocusedColumn {
    /// Focusing a column will not center the column.
    #[default]
    Never,
    /// The focused column will always be centered.
    Always,
    /// Focusing a column will center it if it doesn't fit on the screen together with the
    /// previously focused column.
    OnOverflow,
}

impl<S> knus::Decode<S> for DefaultPresetSize
where
    S: knus::traits::ErrorSpan,
{
    fn decode_node(
        node: &knus::ast::SpannedNode<S>,
        ctx: &mut knus::decode::Context<S>,
    ) -> Result<Self, DecodeError<S>> {
        expect_only_children(node, ctx);

        let mut children = node.children();

        if let Some(child) = children.next() {
            if let Some(unwanted_child) = children.next() {
                ctx.emit_error(DecodeError::unexpected(
                    unwanted_child,
                    "node",
                    "expected no more than one child",
                ));
            }
            PresetSize::decode_node(child, ctx).map(Some).map(Self)
        } else {
            Ok(Self(None))
        }
    }
}
