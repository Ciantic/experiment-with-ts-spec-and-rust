//! Field presence for structural models: a field is either supplied or omitted.

/// One field of a partial structural model.
///
/// A nullable field nests `Option`: `Set(None)` supplies a null, while `Unset` omits the field.
///
/// On the wire a `Set` is the value itself and an `Unset` is absent, so a struct field carries
/// `#[serde(default, skip_serializing_if = "Partial::is_unset")]` and a present key always reads as `Set`.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Partial<T> {
    /// The field is supplied.
    Set(T),
    /// The field is omitted.
    #[default]
    Unset,
}

impl<T> Partial<T> {
    /// Builds a supplied field.
    pub const fn set(value: T) -> Self {
        Self::Set(value)
    }

    /// Builds an omitted field.
    pub const fn unset() -> Self {
        Self::Unset
    }

    /// Reports whether the field is supplied.
    pub const fn is_set(&self) -> bool {
        matches!(self, Self::Set(_))
    }

    /// Reports whether the field is omitted.
    pub const fn is_unset(&self) -> bool {
        matches!(self, Self::Unset)
    }

    /// Borrows the supplied value.
    pub const fn get(&self) -> Option<&T> {
        match self {
            Self::Set(value) => Some(value),
            Self::Unset => None,
        }
    }

    /// Mutably borrows the supplied value.
    pub fn get_mut(&mut self) -> Option<&mut T> {
        match self {
            Self::Set(value) => Some(value),
            Self::Unset => None,
        }
    }

    /// Converts into the supplied value.
    pub fn into_set(self) -> Option<T> {
        match self {
            Self::Set(value) => Some(value),
            Self::Unset => None,
        }
    }

    /// Borrows the field as a structural value.
    #[must_use]
    pub const fn as_ref(&self) -> Partial<&T> {
        match self {
            Self::Set(value) => Partial::Set(value),
            Self::Unset => Partial::Unset,
        }
    }

    /// Mutably borrows the field as a structural value.
    #[must_use]
    pub fn as_mut(&mut self) -> Partial<&mut T> {
        match self {
            Self::Set(value) => Partial::Set(value),
            Self::Unset => Partial::Unset,
        }
    }

    /// Omits the field and returns its previous state.
    pub fn take(&mut self) -> Self {
        std::mem::take(self)
    }

    /// Supplies a value and returns the previous state.
    pub fn replace(&mut self, value: T) -> Self {
        std::mem::replace(self, Self::Set(value))
    }

    /// Supplies a value and returns the one it replaced.
    pub fn insert(&mut self, value: T) -> Option<T> {
        self.replace(value).into_set()
    }

    /// Returns the supplied value, or `default` when the field is omitted.
    pub fn unwrap_or(self, default: T) -> T {
        self.into_set().unwrap_or(default)
    }

    /// Returns the supplied value, or one built by `default` when the field is omitted.
    pub fn unwrap_or_else(self, default: impl FnOnce() -> T) -> T {
        self.into_set().unwrap_or_else(default)
    }

    /// Returns the supplied value, or `T::default()` when the field is omitted.
    pub fn unwrap_or_default(self) -> T
    where
        T: Default,
    {
        self.into_set().unwrap_or_default()
    }

    /// Maps the supplied value, keeping an omitted field omitted.
    #[must_use]
    pub fn map<U>(self, map: impl FnOnce(T) -> U) -> Partial<U> {
        match self {
            Self::Set(value) => Partial::Set(map(value)),
            Self::Unset => Partial::Unset,
        }
    }

    /// Maps the supplied value, or returns `default` when the field is omitted.
    pub fn map_or<U>(self, default: U, map: impl FnOnce(T) -> U) -> U {
        self.into_set().map_or(default, map)
    }

    /// Keeps the field only when its supplied value satisfies `predicate`.
    #[must_use]
    pub fn filter(self, predicate: impl FnOnce(&T) -> bool) -> Self {
        match self {
            Self::Set(value) if predicate(&value) => Self::Set(value),
            _ => Self::Unset,
        }
    }
}

impl<T> Partial<Option<T>> {
    /// Supplies a null, which clears a nullable field.
    pub const fn clear() -> Self {
        Self::Set(None)
    }
}

impl<T> From<T> for Partial<T> {
    fn from(value: T) -> Self {
        Self::Set(value)
    }
}

impl<T> From<Partial<T>> for Option<T> {
    fn from(partial: Partial<T>) -> Self {
        partial.into_set()
    }
}

impl<T> serde::Serialize for Partial<T>
where
    T: serde::Serialize,
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            Self::Set(value) => value.serialize(serializer),
            Self::Unset => serializer.serialize_none(),
        }
    }
}

impl<'de, T> serde::Deserialize<'de> for Partial<T>
where
    T: serde::Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        T::deserialize(deserializer).map(Self::Set)
    }
}

#[cfg(test)]
mod tests {
    use super::Partial;

    #[derive(Debug, PartialEq, serde::Deserialize, serde::Serialize)]
    struct Widget {
        #[serde(default, skip_serializing_if = "Partial::is_unset")]
        name: Partial<String>,
        #[serde(default, skip_serializing_if = "Partial::is_unset")]
        notes: Partial<Option<String>>,
    }

    impl Default for Widget {
        fn default() -> Self {
            Self {
                name: Partial::unset(),
                notes: Partial::unset(),
            }
        }
    }

    #[test]
    fn defaults_to_unset() {
        let partial = Partial::<u8>::default();

        assert!(partial.is_unset());
        assert!(!partial.is_set());
        assert_eq!(partial.get(), None);
    }

    #[test]
    fn set_supplies_a_value() {
        let partial = Partial::set(7);

        assert!(partial.is_set());
        assert!(!partial.is_unset());
        assert_eq!(partial.get(), Some(&7));
    }

    #[test]
    fn omitted_nullable_field_differs_from_a_supplied_null() {
        assert_eq!(Partial::<Option<u8>>::unset().get(), None);
        assert_eq!(Partial::<Option<u8>>::clear().get(), Some(&None));
    }

    #[test]
    fn map_preserves_omission() {
        assert_eq!(Partial::<u8>::unset().map(u16::from), Partial::Unset);
        assert_eq!(Partial::set(7u8).map(u16::from), Partial::Set(7u16));
    }

    #[test]
    fn filter_drops_a_rejected_value() {
        assert_eq!(Partial::set(7).filter(|value| *value > 7), Partial::Unset);
        assert_eq!(Partial::set(7).filter(|value| *value == 7), Partial::Set(7));
        assert_eq!(Partial::<u8>::unset().filter(|_| true), Partial::Unset);
    }

    #[test]
    fn take_and_replace_report_the_previous_state() {
        let mut partial = Partial::set(7);

        let previous = partial.replace(9);
        assert_eq!(previous, Partial::Set(7));
        assert_eq!(partial, Partial::Set(9));

        let previous = partial.take();
        assert_eq!(previous, Partial::Set(9));
        assert_eq!(partial, Partial::Unset);
    }

    #[test]
    fn conversions_treat_a_value_as_supplied() {
        assert_eq!(Partial::from(7), Partial::Set(7));
        assert_eq!(Option::from(Partial::set(7)), Some(7));
        assert_eq!(Option::<u8>::from(Partial::<u8>::unset()), None);
    }

    #[test]
    fn a_supplied_field_serializes_as_its_value() {
        assert_eq!(serde_json::to_string(&Partial::set(7)).unwrap(), "7");
        assert_eq!(
            serde_json::to_string(&Partial::<u8>::unset()).unwrap(),
            "null"
        );
    }

    #[test]
    fn an_omitted_field_is_absent_from_the_struct() {
        assert_eq!(serde_json::to_string(&Widget::default()).unwrap(), "{}");

        let widget = Widget {
            name: Partial::set("Account".to_owned()),
            notes: Partial::clear(),
        };
        assert_eq!(
            serde_json::to_string(&widget).unwrap(),
            r#"{"name":"Account","notes":null}"#
        );
    }

    #[test]
    fn a_present_key_reads_as_supplied() {
        let widget: Widget = serde_json::from_str(r#"{"name":"Account"}"#).unwrap();

        assert_eq!(widget.name, Partial::set("Account".to_owned()));
        assert_eq!(widget.notes, Partial::unset());
    }

    #[test]
    fn a_supplied_null_clears_a_nullable_field() {
        let widget: Widget = serde_json::from_str(r#"{"notes":null}"#).unwrap();

        assert_eq!(widget.name, Partial::unset());
        assert_eq!(widget.notes, Partial::clear());
    }
}
