// We need to implement RenderElement manually due to AsGlesFrame requirement.
// This macro does it for us.
#[macro_export]
macro_rules! niri_render_elements {
    // The two callable variants: with <R> and without <R>. They include From impls because nested
    // repetitions ($type and $variant with + and $R with ?) don't work properly.
    ($name:ident<R> => { $($variant:ident = $type:ty),+ $(,)? }) => {
        $crate::niri_render_elements!(@impl $name () ($name<R>) => { $($variant = $type),+ });

        $(impl<R: $crate::render_helpers::renderer::NiriRenderer> From<$type> for $name<R> {
            fn from(x: $type) -> Self {
                Self::$variant(x)
            }
        })+
    };

    ($name:ident => { $($variant:ident = $type:ty),+ $(,)? }) => {
        $crate::niri_render_elements!(@impl $name ($name) () => { $($variant = $type),+ });

        $(impl From<$type> for $name {
            fn from(x: $type) -> Self {
                Self::$variant(x)
            }
        })+
    };

    // The internal variant that generates most of the code. $name_no_R and $name_R are necessary
    // for the impl RenderElement<SomeRenderer> for $name<SomeRenderer>: since $R does not appear
    // in this line, we cannot condition based on $R like elsewhere, so we condition on duplicate
    // names instead. Like this: $($name_R<SomeRenderer>)? $($name_no_R)? so only one is chosen.
    (@impl $name:ident ($($name_no_R:ident)?) ($($name_R:ident<$R:ident>)?) => { $($variant:ident = $type:ty),+ }) => {
        #[allow(clippy::large_enum_variant)]
        #[derive(Debug)]
        pub enum $name$(<$R: $crate::render_helpers::renderer::NiriRenderer>)? {
            $($variant($type)),+
        }

        impl$(<$R: $crate::render_helpers::renderer::NiriRenderer>)? smithay::backend::renderer::element::Element for $name$(<$R>)? {
            fn id(&self) -> &smithay::backend::renderer::element::Id {
                match self {
                    $($name::$variant(elem) => elem.id()),+
                }
            }

            fn current_commit(&self) -> smithay::backend::renderer::utils::CommitCounter {
                match self {
                    $($name::$variant(elem) => elem.current_commit()),+
                }
            }

            fn geometry(&self, scale: smithay::utils::Scale<f64>) -> smithay::utils::Rectangle<i32, smithay::utils::Physical> {
                match self {
                    $($name::$variant(elem) => elem.geometry(scale)),+
                }
            }

            fn transform(&self) -> smithay::utils::Transform {
                match self {
                    $($name::$variant(elem) => elem.transform()),+
                }
            }

            fn src(&self) -> smithay::utils::Rectangle<f64, smithay::utils::Buffer> {
                match self {
                    $($name::$variant(elem) => elem.src()),+
                }
            }

            fn damage_since(
                &self,
                scale: smithay::utils::Scale<f64>,
                commit: Option<smithay::backend::renderer::utils::CommitCounter>,
            ) -> smithay::backend::renderer::utils::DamageSet<i32, smithay::utils::Physical> {
                match self {
                    $($name::$variant(elem) => elem.damage_since(scale, commit)),+
                }
            }

            fn opaque_regions(&self, scale: smithay::utils::Scale<f64>) -> smithay::backend::renderer::utils::OpaqueRegions<i32, smithay::utils::Physical> {
                match self {
                    $($name::$variant(elem) => elem.opaque_regions(scale)),+
                }
            }

            fn alpha(&self) -> f32 {
                match self {
                    $($name::$variant(elem) => elem.alpha()),+
                }
            }

            fn kind(&self) -> smithay::backend::renderer::element::Kind {
                match self {
                    $($name::$variant(elem) => elem.kind()),+
                }
            }

            fn is_framebuffer_effect(&self) -> bool {
                match self {
                    $($name::$variant(elem) => elem.is_framebuffer_effect()),+
                }
            }
        }

        impl smithay::backend::renderer::element::RenderElement<smithay::backend::renderer::gles::GlesRenderer>
            for $($name_R<smithay::backend::renderer::gles::GlesRenderer>)? $($name_no_R)?
        {
            fn capture_framebuffer(
                &self,
                frame: &mut smithay::backend::renderer::gles::GlesFrame<'_, '_>,
                src: smithay::utils::Rectangle<f64, smithay::utils::Buffer>,
                dst: smithay::utils::Rectangle<i32, smithay::utils::Physical>,
                cache: &smithay::utils::user_data::UserDataMap,
            ) -> Result<(), smithay::backend::renderer::gles::GlesError> {
                match self {
                    $($name::$variant(elem) => {
                        smithay::backend::renderer::element::RenderElement::<smithay::backend::renderer::gles::GlesRenderer>::capture_framebuffer(elem, frame, src, dst, cache)
                    })+
                }
            }

            fn draw(
                &self,
                frame: &mut smithay::backend::renderer::gles::GlesFrame<'_, '_>,
                src: smithay::utils::Rectangle<f64, smithay::utils::Buffer>,
                dst: smithay::utils::Rectangle<i32, smithay::utils::Physical>,
                damage: &[smithay::utils::Rectangle<i32, smithay::utils::Physical>],
                opaque_regions: &[smithay::utils::Rectangle<i32, smithay::utils::Physical>],
                cache: Option<&smithay::utils::user_data::UserDataMap>,
            ) -> Result<(), smithay::backend::renderer::gles::GlesError> {
                match self {
                    $($name::$variant(elem) => {
                        smithay::backend::renderer::element::RenderElement::<smithay::backend::renderer::gles::GlesRenderer>::draw(elem, frame, src, dst, damage, opaque_regions, cache)
                    })+
                }
            }

            fn underlying_storage(&self, renderer: &mut smithay::backend::renderer::gles::GlesRenderer) -> Option<smithay::backend::renderer::element::UnderlyingStorage<'_>> {
                match self {
                    $($name::$variant(elem) => elem.underlying_storage(renderer)),+
                }
            }
        }

        impl<'render> smithay::backend::renderer::element::RenderElement<$crate::backend::tty::TtyRenderer<'render>>
            for $($name_R<$crate::backend::tty::TtyRenderer<'render>>)? $($name_no_R)?
        {
            fn capture_framebuffer(
                &self,
                frame: &mut $crate::backend::tty::TtyFrame<'render, '_, '_>,
                src: smithay::utils::Rectangle<f64, smithay::utils::Buffer>,
                dst: smithay::utils::Rectangle<i32, smithay::utils::Physical>,
                cache: &smithay::utils::user_data::UserDataMap,
            ) -> Result<(), $crate::backend::tty::TtyRendererError<'render>> {
                match self {
                    $($name::$variant(elem) => {
                        smithay::backend::renderer::element::RenderElement::<$crate::backend::tty::TtyRenderer<'render>>::capture_framebuffer(elem, frame, src, dst, cache)
                    })+
                }
            }

            fn draw(
                &self,
                frame: &mut $crate::backend::tty::TtyFrame<'render, '_, '_>,
                src: smithay::utils::Rectangle<f64, smithay::utils::Buffer>,
                dst: smithay::utils::Rectangle<i32, smithay::utils::Physical>,
                damage: &[smithay::utils::Rectangle<i32, smithay::utils::Physical>],
                opaque_regions: &[smithay::utils::Rectangle<i32, smithay::utils::Physical>],
                cache: Option<&smithay::utils::user_data::UserDataMap>,
            ) -> Result<(), $crate::backend::tty::TtyRendererError<'render>> {
                match self {
                    $($name::$variant(elem) => {
                        smithay::backend::renderer::element::RenderElement::<$crate::backend::tty::TtyRenderer<'render>>::draw(elem, frame, src, dst, damage, opaque_regions, cache)
                    })+
                }
            }

            fn underlying_storage(
                &self,
                renderer: &mut $crate::backend::tty::TtyRenderer<'render>,
            ) -> Option<smithay::backend::renderer::element::UnderlyingStorage<'_>> {
                match self {
                    $($name::$variant(elem) => elem.underlying_storage(renderer)),+
                }
            }
        }
    };
}

use smithay::backend::renderer::element::utils::RescaleRenderElement;
use smithay::backend::renderer::element::{Element, Id, Kind, RenderElement};
use smithay::backend::renderer::utils::{CommitCounter, DamageSet, OpaqueRegions};
use smithay::backend::renderer::Renderer;
use smithay::utils::user_data::UserDataMap;
use smithay::utils::{Buffer, Physical, Rectangle, Scale, Transform};

/// A [`RescaleRenderElement`] with a manual [`Debug`] impl.
///
/// Some elements (like [`WaylandSurfaceRenderElement`]) don't implement `Debug`, which
/// prevents them from being used directly in the generated render element enums.
pub struct RescaledElement<E> {
    inner: RescaleRenderElement<E>,
}

impl<E> RescaledElement<E> {
    pub fn new(inner: RescaleRenderElement<E>) -> Self {
        Self { inner }
    }
}

impl<E> std::fmt::Debug for RescaledElement<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RescaledElement").finish_non_exhaustive()
    }
}

impl<E: Element> Element for RescaledElement<E> {
    fn id(&self) -> &Id {
        self.inner.id()
    }

    fn current_commit(&self) -> CommitCounter {
        self.inner.current_commit()
    }

    fn location(&self, scale: Scale<f64>) -> smithay::utils::Point<i32, Physical> {
        self.inner.location(scale)
    }

    fn src(&self) -> Rectangle<f64, Buffer> {
        self.inner.src()
    }

    fn transform(&self) -> Transform {
        self.inner.transform()
    }

    fn geometry(&self, scale: Scale<f64>) -> Rectangle<i32, Physical> {
        self.inner.geometry(scale)
    }

    fn damage_since(
        &self,
        scale: Scale<f64>,
        commit: Option<CommitCounter>,
    ) -> DamageSet<i32, Physical> {
        self.inner.damage_since(scale, commit)
    }

    fn opaque_regions(&self, scale: Scale<f64>) -> OpaqueRegions<i32, Physical> {
        self.inner.opaque_regions(scale)
    }

    fn alpha(&self) -> f32 {
        self.inner.alpha()
    }

    fn kind(&self) -> Kind {
        self.inner.kind()
    }

    fn is_framebuffer_effect(&self) -> bool {
        self.inner.is_framebuffer_effect()
    }
}

impl<R: Renderer, E: RenderElement<R>> RenderElement<R> for RescaledElement<E> {
    fn draw(
        &self,
        frame: &mut R::Frame<'_, '_>,
        src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque_regions: &[Rectangle<i32, Physical>],
        cache: Option<&UserDataMap>,
    ) -> Result<(), R::Error> {
        self.inner
            .draw(frame, src, dst, damage, opaque_regions, cache)
    }
}
