use super::*;

pub(crate) trait Tally {
  fn tally(self, count: usize) -> Tallied;
}

impl Tally for &'static str {
  fn tally(self, count: usize) -> Tallied {
    Tallied { noun: self, count }
  }
}

pub(crate) struct Tallied {
  count: usize,
  noun: &'static str,
}

impl Display for Tallied {
  fn fmt(&self, f: &mut Formatter) -> fmt::Result {
    if self.count == 1 {
      write!(f, "{} {}", self.count, self.noun)
    } else {
      write!(f, "{} {}s", self.count, self.noun)
    }
  }
}
