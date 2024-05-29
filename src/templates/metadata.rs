use super::*;

pub(crate) struct MetadataHtml<'a>(pub &'a Value);

impl<'a> Display for MetadataHtml<'a> {
  fn fmt(&self, f: &mut Formatter) -> fmt::Result {
    match self.0 {
      Value::Array(x) => {
        write!(f, "<ul>")?;
        for element in x {
          write!(f, "<li>{}</li>", MetadataHtml(element))?;
        }
        write!(f, "</ul>")
      }
      Value::Bool(x) => write!(f, "{x}"),
      Value::Bytes(x) => {
        for byte in x {
          write!(f, "{byte:02X}")?;
        }
        Ok(())
      }
      Value::Float(x) => write!(f, "{x}"),
      Value::Integer(x) => write!(f, "{}", i128::from(*x)),
      Value::Map(x) => {
        write!(f, "<dl>")?;
        for (key, value) in x {
          write!(f, "<dt>{}</dt>", MetadataHtml(key))?;
          write!(f, "<dd>{}</dd>", MetadataHtml(value))?;
        }
        write!(f, "</dl>")
      }
      Value::Null => write!(f, "null"),
      Value::Tag(tag, value) => write!(f, "<sup>{tag}</sup>{}", MetadataHtml(value)),
      Value::Text(x) => x.escape(f, false),
      _ => write!(f, "unknown"),
    }
  }
}


