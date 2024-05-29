use super::*;

#[derive(Copy, Clone, Debug, PartialEq, Ord, PartialOrd, Eq)]
pub struct SpacedRune {
  pub(crate) rune: Rune,
  pub(crate) spacers: u32,
}

impl FromStr for SpacedRune {
  type Err = Error;

  fn from_str(s: &str) -> Result<Self, Self::Err> {
    let mut rune = String::new();
    let mut spacers = 0u32;

    for c in s.chars() {
      match c {
        'A'..='Z' => rune.push(c),
        '.' | '•' => {
          let flag = 1 << rune.len().checked_sub(1).context("leading spacer")?;
          if spacers & flag != 0 {
            bail!("double spacer");
          }
          spacers |= flag;
        }
        _ => bail!("invalid character"),
      }
    }

    if 32 - spacers.leading_zeros() >= rune.len().try_into().unwrap() {
      bail!("trailing spacer")
    }

    Ok(SpacedRune {
      rune: rune.parse()?,
      spacers,
    })
  }
}

impl Display for SpacedRune {
  fn fmt(&self, f: &mut Formatter) -> fmt::Result {
    let rune = self.rune.to_string();

    for (i, c) in rune.chars().enumerate() {
      write!(f, "{c}")?;

      if i < rune.len() - 1 && self.spacers & 1 << i != 0 {
        write!(f, "•")?;
      }
    }

    Ok(())
  }
}

impl Serialize for SpacedRune {
  fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
  where
    S: Serializer,
  {
    serializer.collect_str(self)
  }
}

impl<'de> Deserialize<'de> for SpacedRune {
  fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
  where
    D: Deserializer<'de>,
  {
    DeserializeFromStr::with(deserializer)
  }
}
