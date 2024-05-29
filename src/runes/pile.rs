use super::*;

#[derive(Debug, Deserialize, Serialize, PartialEq, Clone, Copy)]
pub struct Pile {
  pub amount: u128,
  pub divisibility: u8,
  pub symbol: Option<char>,
}

impl Display for Pile {
  fn fmt(&self, f: &mut Formatter) -> fmt::Result {
    let cutoff = 10u128.pow(self.divisibility.into());

    let whole = self.amount / cutoff;
    let mut fractional = self.amount % cutoff;

    if fractional == 0 {
      write!(f, "{whole}")?;
    } else {
      let mut width = usize::from(self.divisibility);
      while fractional % 10 == 0 {
        fractional /= 10;
        width -= 1;
      }

      write!(f, "{whole}.{fractional:0>width$}")?;
    }

    if let Some(symbol) = self.symbol {
      write!(f, "\u{00A0}{symbol}")?;
    }

    Ok(())
  }
}
