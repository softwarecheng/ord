use super::*;

#[derive(Copy, Clone, Debug)]
pub(super) enum Tag {
    Body = 0,
    Flags = 2,
    Rune = 4,
    Limit = 6,
    Term = 8,
    Deadline = 10,
    DefaultOutput = 12,
    Claim = 14,
    #[allow(unused)]
    Burn = 126,

    Divisibility = 1,
    Spacers = 3,
    Symbol = 5,
    #[allow(unused)]
    Nop = 127,
}

impl Tag {
    pub(super) fn take(self, fields: &mut HashMap<u128, u128>) -> Option<u128> {
        fields.remove(&self.into())
    }
}

impl From<Tag> for u128 {
    fn from(tag: Tag) -> Self {
        tag as u128
    }
}

impl PartialEq<u128> for Tag {
    fn eq(&self, other: &u128) -> bool {
        u128::from(*self) == *other
    }
}
