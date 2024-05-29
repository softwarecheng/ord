use super::*;

#[derive(Debug, PartialEq, Copy, Clone)]
pub struct Decimal {
    value: u128,
    scale: u8,
}

impl Display for Decimal {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        let magnitude = 10u128.pow(self.scale.into());

        let integer = self.value / magnitude;
        let mut fraction = self.value % magnitude;

        write!(f, "{integer}")?;

        if fraction > 0 {
            let mut width = self.scale.into();

            while fraction % 10 == 0 {
                fraction /= 10;
                width -= 1;
            }

            write!(f, ".{fraction:0>width$}", width = width)?;
        }

        Ok(())
    }
}

impl FromStr for Decimal {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Some((integer, decimal)) = s.split_once('.') {
            if integer.is_empty() && decimal.is_empty() {
                bail!("empty decimal");
            }

            let integer = if integer.is_empty() {
                0
            } else {
                integer.parse::<u128>()?
            };

            let (decimal, scale) = if decimal.is_empty() {
                (0, 0)
            } else {
                let trailing_zeros = decimal.chars().rev().take_while(|c| *c == '0').count();
                let significant_digits = decimal.chars().count() - trailing_zeros;
                let decimal =
                    decimal.parse::<u128>()? / 10u128.pow(u32::try_from(trailing_zeros).unwrap());
                (decimal, u8::try_from(significant_digits).unwrap())
            };

            Ok(Self {
                value: integer * 10u128.pow(u32::from(scale)) + decimal,
                scale,
            })
        } else {
            Ok(Self {
                value: s.parse::<u128>()?,
                scale: 0,
            })
        }
    }
}

impl Serialize for Decimal {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Decimal {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        DeserializeFromStr::with(deserializer)
    }
}
