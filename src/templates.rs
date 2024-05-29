use {super::*, boilerplate::Boilerplate};

pub(crate) use {
  block::BlockHtml,
  children::ChildrenHtml,
  clock::ClockSvg,
  collections::CollectionsHtml,
  home::HomeHtml,
  iframe::Iframe,
  input::InputHtml,
  inscription::InscriptionHtml,
  inscriptions::InscriptionsHtml,
  inscriptions_block::InscriptionsBlockHtml,
  metadata::MetadataHtml,
  output::OutputHtml,
  preview::{
    PreviewAudioHtml, PreviewCodeHtml, PreviewFontHtml, PreviewImageHtml, PreviewMarkdownHtml,
    PreviewModelHtml, PreviewPdfHtml, PreviewTextHtml, PreviewUnknownHtml, PreviewVideoHtml,
  },
  range::RangeHtml,
  rare::RareTxt,
  rune_balances::RuneBalancesHtml,
  sat::SatHtml,
  server_config::ServerConfig,
};

pub use {
  blocks::BlocksHtml, rune::RuneHtml, runes::RunesHtml, status::StatusHtml,
  transaction::TransactionHtml,
};

pub mod block;
pub mod blocks;
mod children;
mod clock;
pub mod collections;
mod home;
mod iframe;
mod input;
pub mod inscription;
pub mod inscriptions;
mod inscriptions_block;
mod metadata;
pub mod output;
mod preview;
mod range;
mod rare;
pub mod rune;
pub mod rune_balances;
pub mod runes;
pub mod sat;
pub mod status;
pub mod transaction;

#[derive(Boilerplate)]
pub(crate) struct PageHtml<T: PageContent> {
  content: T,
  config: Arc<ServerConfig>,
}

impl<T> PageHtml<T>
where
  T: PageContent,
{
  pub(crate) fn new(content: T, config: Arc<ServerConfig>) -> Self {
    Self { content, config }
  }

  fn og_image(&self) -> String {
    if let Some(domain) = &self.config.domain {
      format!("https://{domain}/static/favicon.png")
    } else {
      "https://ordinals.com/static/favicon.png".into()
    }
  }

  fn superscript(&self) -> String {
    if self.config.chain == Chain::Mainnet {
      "beta".into()
    } else {
      self.config.chain.to_string()
    }
  }
}

pub(crate) trait PageContent: Display + 'static {
  fn title(&self) -> String;

  fn page(self, server_config: Arc<ServerConfig>) -> PageHtml<Self>
  where
    Self: Sized,
  {
    PageHtml::new(self, server_config)
  }

  fn preview_image_url(&self) -> Option<Trusted<String>> {
    None
  }
}
