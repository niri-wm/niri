pub trait MergeWith<T> {
    fn merge_with(&mut self, part: &T);

    fn merged_with(mut self, part: &T) -> Self
    where
        Self: Sized,
    {
        self.merge_with(part);
        self
    }

    fn from_part(part: &T) -> Self
    where
        Self: Default + Sized,
    {
        Self::default().merged_with(part)
    }
}
