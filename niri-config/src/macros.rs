macro_rules! merge {
    (($self:expr, $part:expr), $($field:ident),+ $(,)*) => {
        $(
            if let Some(x) = &$part.$field {
                $self.$field.merge_with(x);
            }
        )+
    };
}

macro_rules! merge_clone {
    (($self:expr, $part:expr), $($field:ident),+ $(,)*) => {
        $(
            if let Some(x) = &$part.$field {
                $self.$field.clone_from(x);
            }
        )+
    };
}

macro_rules! merge_clone_opt {
    (($self:expr, $part:expr), $($field:ident),+ $(,)*) => {
        $(
            if $part.$field.is_some() {
                $self.$field.clone_from(&$part.$field);
            }
        )+
    };
}

macro_rules! merge_color_gradient {
    (($self:expr, $part:expr), $(($color:ident, $gradient:ident)),+ $(,)*) => {
        $(
            if let Some(x) = $part.$color {
                $self.$color = x;
                $self.$gradient = None;
            }
            if let Some(x) = $part.$gradient {
                $self.$gradient = Some(x);
            }
        )+
    };
}

macro_rules! merge_color_gradient_opt {
    (($self:expr, $part:expr), $(($color:ident, $gradient:ident)),+ $(,)*) => {
        $(
            if let Some(x) = $part.$color {
                $self.$color = Some(x);
                $self.$gradient = None;
            }
            if let Some(x) = $part.$gradient {
                $self.$gradient = Some(x);
            }
        )+
    };
}

macro_rules! merge_on_off {
    (($self:expr, $part:expr)) => {
        if $part.off {
            $self.off = true;
            $self.on = false;
        }

        if $part.on {
            $self.off = false;
            $self.on = true;
        }
    };
}
