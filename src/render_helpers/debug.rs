use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::backend::renderer::element::{Element, Id, Kind};
use smithay::backend::renderer::utils::CommitCounter;
use smithay::backend::renderer::Color32F;
use smithay::utils::Scale;

use super::renderer::NiriRenderer;
use super::solid_color::SolidColorRenderElement;
use crate::niri::OutputRenderElements;

pub fn push_opaque_regions<R: NiriRenderer>(
    elem: &OutputRenderElements<R>,
    scale: Scale<f64>,
    push: &mut dyn FnMut(OutputRenderElements<R>),
) {
    // HACK
    if format!("{elem:?}").contains("ExtraDamage") {
        return;
    }

    let geo = elem.geometry(scale);
    let mut opaque = elem.opaque_regions(scale).to_vec();

    for rect in &mut opaque {
        rect.loc += geo.loc;
    }

    let semitransparent = geo.subtract_rects(opaque.iter().copied());

    for rect in opaque {
        let color = SolidColorRenderElement::new(
            Id::new(),
            rect.to_f64().to_logical(scale),
            CommitCounter::default(),
            Color32F::from([0., 0., 0.2, 0.2]),
            Kind::Unspecified,
        );
        push(color.into());
    }

    for rect in semitransparent {
        let color = SolidColorRenderElement::new(
            Id::new(),
            rect.to_f64().to_logical(scale),
            CommitCounter::default(),
            Color32F::from([0.3, 0., 0., 0.3]),
            Kind::Unspecified,
        );
        push(color.into());
    }
}

pub fn draw_damage<R: NiriRenderer>(
    damage_tracker: &mut OutputDamageTracker,
    elements: &mut Vec<OutputRenderElements<R>>,
) {
    let _span = tracy_client::span!("draw_damage");

    let Ok((_, scale, _)) = damage_tracker.mode().try_into() else {
        return;
    };

    let Ok((Some(damage), _)) = damage_tracker.damage_output(1, elements) else {
        return;
    };

    for rect in damage {
        let color = SolidColorRenderElement::new(
            Id::new(),
            rect.to_f64().to_logical(scale),
            CommitCounter::default(),
            Color32F::from([0.3, 0., 0., 0.3]),
            Kind::Unspecified,
        );
        elements.insert(0, OutputRenderElements::SolidColor(color));
    }
}
