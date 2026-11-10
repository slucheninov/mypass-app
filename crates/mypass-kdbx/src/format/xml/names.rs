//! Single source of truth for parent-specific decoder dispatch and raw-extra exclusions.
macro_rules! elements {
    ($kind:ident { $($element:ident => $wire:literal),+ $(,)? }) => {
        pub(super) enum $kind { $($element),+ }
        impl $kind {
            pub(super) fn from_name(name: &str) -> Option<Self> {
                match name {
                    $($wire => Some(Self::$element),)+
                    _ => None,
                }
            }
        }
    };
}
elements!(MetaElement {
    Generator => "Generator",
    DatabaseName => "DatabaseName",
    DatabaseDescription => "DatabaseDescription",
    DefaultUserName => "DefaultUserName",
    HistoryMaxItems => "HistoryMaxItems",
    HistoryMaxSize => "HistoryMaxSize",
    RecycleBinEnabled => "RecycleBinEnabled",
    RecycleBinUUID => "RecycleBinUUID",
    CustomData => "CustomData",
    Binary => "Binary",
});
elements!(GroupElement {
    Uuid => "UUID",
    Name => "Name",
    IconID => "IconID",
    Times => "Times",
    Entry => "Entry",
    Group => "Group",
    CustomData => "CustomData",
    Binary => "Binary",
});
elements!(EntryElement {
    Uuid => "UUID",
    IconID => "IconID",
    String => "String",
    StringArray => "StringArray",
    Binary => "Binary",
    Times => "Times",
    CustomData => "CustomData",
    History => "History",
});

#[derive(Clone, Copy)]
pub(crate) enum ParentKind {
    Meta,
    Group,
    Entry,
}
impl ParentKind {
    pub(super) fn is_known_element(self, name: &str) -> bool {
        match self {
            Self::Meta => MetaElement::from_name(name).is_some(),
            Self::Group => GroupElement::from_name(name).is_some(),
            Self::Entry => EntryElement::from_name(name).is_some(),
        }
    }
}
