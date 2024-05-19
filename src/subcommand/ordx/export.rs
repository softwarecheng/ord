use super::*;

#[derive(Debug, Parser)]
pub(crate) struct Export {
  #[arg(long, help = "export filename path")]
  filename: String,
  #[arg(long, help = "use parallel mode (need more memory)")]
  parallel: bool,
  #[arg(long, help = "cache for inscriptions")]
  cache: u64,
}

impl Export {
  pub(crate) fn run(self, settings: Settings) -> SubcommandResult {
    let index = Index::open(&settings)?;

    index.update()?;
    index.export_ordx(
      &self.filename,
      self.cache,
      self.parallel,
      settings.chain(),
      settings.first_inscription_height(),
    )?;

    Ok(None)
  }
}
