use super::*;

#[derive(Debug, Parser)]
pub(crate) struct Export {
  #[arg(long, help = "export filename path")]
  filename: String,
  #[arg(long, help = "cache inscription count")]
  cache: u64,
}

impl Export {
  pub(crate) fn run(self, settings: Settings) -> SubcommandResult {
    let index = Index::open(&settings)?;

    index.update()?;
    if self.cache <= 0 {
      bail!("cache must be greater than 0");
    }
    index.export_ordx(
      &self.filename,
      self.cache,
      settings.chain(),
      settings.first_inscription_height(),
    )?;

    Ok(None)
  }
}
