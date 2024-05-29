use {super::*, std::num::TryFromIntError};

#[derive(Debug, PartialEq, Copy, Clone, Hash, Eq, Ord, PartialOrd)]
pub struct RuneId {
  pub height: u32,
  pub index: u16,
}

impl TryFrom<u128> for RuneId {
  type Error = TryFromIntError;

  fn try_from(n: u128) -> Result<Self, Self::Error> {
    Ok(Self {
      height: u32::try_from(n >> 16)?,
      index: u16::try_from(n & 0xFFFF).unwrap(),
    })
  }
}

impl From<RuneId> for u128 {
  fn from(id: RuneId) -> Self {
    u128::from(id.height) << 16 | u128::from(id.index)
  }
}

impl Display for RuneId {
  fn fmt(&self, f: &mut Formatter) -> fmt::Result {
    write!(f, "{}:{}", self.height, self.index,)
  }
}

impl FromStr for RuneId {
  type Err = Error;

  fn from_str(s: &str) -> Result<Self, Self::Err> {
    let (height, index) = s
      .split_once(':')
      .ok_or_else(|| anyhow!("invalid rune ID: {s}"))?;

    Ok(Self {
      height: height.parse()?,
      index: index.parse()?,
    })
  }
}

impl Serialize for RuneId {
  fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
  where
    S: Serializer,
  {
    serializer.collect_str(self)
  }
}

impl<'de> Deserialize<'de> for RuneId {
  fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
  where
    D: Deserializer<'de>,
  {
    DeserializeFromStr::with(deserializer)
  }
}
