use super::*;

#[derive(Boilerplate, Default)]
pub(crate) struct InscriptionHtml {
  pub(crate) chain: Chain,
  pub(crate) charms: u16,
  pub(crate) children: Vec<InscriptionId>,
  pub(crate) fee: u64,
  pub(crate) height: u32,
  pub(crate) inscription: Inscription,
  pub(crate) id: InscriptionId,
  pub(crate) number: i32,
  pub(crate) next: Option<InscriptionId>,
  pub(crate) output: Option<TxOut>,
  pub(crate) parent: Option<InscriptionId>,
  pub(crate) previous: Option<InscriptionId>,
  pub(crate) rune: Option<SpacedRune>,
  pub(crate) sat: Option<Sat>,
  pub(crate) satpoint: SatPoint,
  pub(crate) timestamp: DateTime<Utc>,
}

impl PageContent for InscriptionHtml {
  fn title(&self) -> String {
    format!("Inscription {}", self.number)
  }

  fn preview_image_url(&self) -> Option<Trusted<String>> {
    Some(Trusted(format!("/content/{}", self.id)))
  }
}


