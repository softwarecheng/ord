use {
    self::{
        accept_encoding::AcceptEncoding,
        accept_json::AcceptJson,
        error::{OptionExt, ServerError, ServerResult},
    },
    super::*,
    crate::{
        server_config::ServerConfig,
        templates::{
            BlockHtml, BlocksHtml, ChildrenHtml, ClockSvg, CollectionsHtml, HomeHtml, InputHtml,
            InscriptionHtml, InscriptionsBlockHtml, InscriptionsHtml, OutputHtml, PageContent,
            PageHtml, PreviewAudioHtml, PreviewCodeHtml, PreviewFontHtml, PreviewImageHtml,
            PreviewMarkdownHtml, PreviewModelHtml, PreviewPdfHtml, PreviewTextHtml,
            PreviewUnknownHtml, PreviewVideoHtml, RangeHtml, RareTxt, RuneBalancesHtml, RuneHtml,
            RunesHtml, SatHtml, TransactionHtml,
        },
    },
    axum::{
        body,
        extract::{Extension, Json, Path, Query},
        http::{header, HeaderMap, HeaderValue, StatusCode, Uri},
        response::{IntoResponse, Redirect, Response},
        routing::get,
        Router,
    },
    axum_server::Handle,
    brotli::Decompressor,
    rayon::prelude::*,
    rust_embed::RustEmbed,
    rustls_acme::{
        acme::{LETS_ENCRYPT_PRODUCTION_DIRECTORY, LETS_ENCRYPT_STAGING_DIRECTORY},
        axum::AxumAcceptor,
        caches::DirCache,
        AcmeConfig,
    },
    std::{cmp::Ordering, io::Read, str, sync::Arc},
    tokio_stream::StreamExt,
    tower_http::{
        compression::CompressionLayer,
        cors::{Any, CorsLayer},
        set_header::SetResponseHeaderLayer,
        validate_request::ValidateRequestHeaderLayer,
    },
};

mod accept_encoding;
mod accept_json;
mod error;
pub(crate) mod query;

enum SpawnConfig {
    Https(AxumAcceptor),
    Http,
    Redirect(String),
}

#[derive(Deserialize)]
struct Search {
    query: String,
}

#[derive(RustEmbed)]
#[folder = "static"]
struct StaticAssets;

struct StaticHtml {
    title: &'static str,
    html: &'static str,
}

impl PageContent for StaticHtml {
    fn title(&self) -> String {
        self.title.into()
    }
}

impl Display for StaticHtml {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        f.write_str(self.html)
    }
}

#[derive(Debug, Parser, Clone)]
pub struct Server {
    #[arg(
        long,
        help = "Listen on <ADDRESS> for incoming requests. [default: 0.0.0.0]"
    )]
    pub(crate) address: Option<String>,
    #[arg(
        long,
        help = "Request ACME TLS certificate for <ACME_DOMAIN>. This ord instance must be reachable at <ACME_DOMAIN>:443 to respond to Let's Encrypt ACME challenges."
    )]
    pub(crate) acme_domain: Vec<String>,
    #[arg(
        long,
        help = "Use <CSP_ORIGIN> in Content-Security-Policy header. Set this to the public-facing URL of your ord instance."
    )]
    pub(crate) csp_origin: Option<String>,
    #[arg(
        long,
        help = "Decompress encoded content. Currently only supports brotli. Be careful using this on production instances. A decompressed inscription may be arbitrarily large, making decompression a DoS vector."
    )]
    pub(crate) decompress: bool,
    #[arg(long, help = "Disable JSON API.")]
    pub(crate) disable_json_api: bool,
    #[arg(
        long,
        help = "Listen on <HTTP_PORT> for incoming HTTP requests. [default: 80]"
    )]
    pub(crate) http_port: Option<u16>,
    #[arg(
        long,
        group = "port",
        help = "Listen on <HTTPS_PORT> for incoming HTTPS requests. [default: 443]"
    )]
    pub(crate) https_port: Option<u16>,
    #[arg(long, help = "Store ACME TLS certificates in <ACME_CACHE>.")]
    pub(crate) acme_cache: Option<PathBuf>,
    #[arg(long, help = "Provide ACME contact <ACME_CONTACT>.")]
    pub(crate) acme_contact: Vec<String>,
    #[arg(long, help = "Serve HTTP traffic on <HTTP_PORT>.")]
    pub(crate) http: bool,
    #[arg(long, help = "Serve HTTPS traffic on <HTTPS_PORT>.")]
    pub(crate) https: bool,
    #[arg(long, help = "Redirect HTTP traffic to HTTPS.")]
    pub(crate) redirect_http_to_https: bool,
    #[arg(long, alias = "nosync", help = "Do not update the index.")]
    pub(crate) no_sync: bool,
    #[arg(
        long,
        help = "Proxy `/content/INSCRIPTION_ID` requests to `<CONTENT_PROXY>/content/INSCRIPTION_ID` if the inscription is not present on current chain."
    )]
    pub(crate) content_proxy: Option<Url>,
    #[arg(
        long,
        default_value = "5s",
        help = "Poll Bitcoin Core every <POLLING_INTERVAL>."
    )]
    pub(crate) polling_interval: humantime::Duration,
}

impl Server {
    pub fn run(self, settings: Settings, index: Arc<Index>, handle: Handle) -> SubcommandResult {
        Runtime::new()?.block_on(async {
            let index_clone = index.clone();

            let index_thread = thread::spawn(move || loop {
                if SHUTTING_DOWN.load(atomic::Ordering::Relaxed) {
                    break;
                }

                if !self.no_sync {
                    if let Err(error) = index_clone.update() {
                        log::warn!("Updating index: {error}");
                    }
                }

                thread::sleep(self.polling_interval.into());
            });

            INDEXER.lock().unwrap().replace(index_thread);

            let settings = Arc::new(settings);
            let acme_domains = self.acme_domains()?;

            let server_config = Arc::new(ServerConfig {
                chain: settings.chain(),
                content_proxy: self.content_proxy.clone(),
                csp_origin: self.csp_origin.clone(),
                decompress: self.decompress,
                domain: acme_domains.first().cloned(),
                index_sats: index.has_sat_index(),
                json_api_enabled: !self.disable_json_api,
            });

            let router = Router::new()
                .route("/", get(Self::home))
                .route("/block/:query", get(Self::block))
                .route("/blockcount", get(Self::block_count))
                .route("/blockhash", get(Self::block_hash))
                .route("/blockhash/:height", get(Self::block_hash_from_height))
                .route("/blockheight", get(Self::block_height))
                .route("/blocks", get(Self::blocks))
                .route("/blocktime", get(Self::block_time))
                .route("/bounties", get(Self::bounties))
                .route("/children/:inscription_id", get(Self::children))
                .route(
                    "/children/:inscription_id/:page",
                    get(Self::children_paginated),
                )
                .route("/clock", get(Self::clock))
                .route("/collections", get(Self::collections))
                .route("/collections/:page", get(Self::collections_paginated))
                .route("/content/:inscription_id", get(Self::content))
                .route("/faq", get(Self::faq))
                .route("/favicon.ico", get(Self::favicon))
                .route("/feed.xml", get(Self::feed))
                .route("/input/:block/:transaction/:input", get(Self::input))
                .route("/inscription/:inscription_query", get(Self::inscription))
                .route("/inscriptions", get(Self::inscriptions))
                .route("/inscriptions/:page", get(Self::inscriptions_paginated))
                .route(
                    "/inscriptions/block/:height",
                    get(Self::inscriptions_in_block),
                )
                .route(
                    "/inscriptions/block/:height/:page",
                    get(Self::inscriptions_in_block_paginated),
                )
                .route("/install.sh", get(Self::install_script))
                .route("/ordinal/:sat", get(Self::ordinal))
                .route("/output/:output", get(Self::output))
                .route("/preview/:inscription_id", get(Self::preview))
                .route("/r/blockhash", get(Self::block_hash_json))
                .route(
                    "/r/blockhash/:height",
                    get(Self::block_hash_from_height_json),
                )
                .route("/r/blockheight", get(Self::block_height))
                .route("/r/blocktime", get(Self::block_time))
                .route("/r/blockinfo/:query", get(Self::block_info))
                .route(
                    "/r/inscription/:inscription_id",
                    get(Self::inscription_recursive),
                )
                .route("/r/children/:inscription_id", get(Self::children_recursive))
                .route(
                    "/r/children/:inscription_id/:page",
                    get(Self::children_recursive_paginated),
                )
                .route("/r/metadata/:inscription_id", get(Self::metadata))
                .route("/r/sat/:sat_number", get(Self::sat_inscriptions))
                .route(
                    "/r/sat/:sat_number/:page",
                    get(Self::sat_inscriptions_paginated),
                )
                .route(
                    "/r/sat/:sat_number/at/:index",
                    get(Self::sat_inscription_at_index),
                )
                .route("/range/:start/:end", get(Self::range))
                .route("/rare.txt", get(Self::rare_txt))
                .route("/rune/:rune", get(Self::rune))
                .route("/runes", get(Self::runes))
                .route("/runes/balances", get(Self::runes_balances))
                .route("/sat/:sat", get(Self::sat))
                .route("/search", get(Self::search_by_query))
                .route("/search/*query", get(Self::search_by_path))
                .route("/static/*path", get(Self::static_asset))
                .route("/status", get(Self::status))
                .route("/tx/:txid", get(Self::transaction))
                //https://github.com/OLProtocol/ordx customization
                .route(
                    "/ordx/block/inscriptions/:height",
                    get(Self::ordx_block_inscriptions),
                )
                .route(
                    "/ordx/block/tx/outputs/inscriptions/:height",
                    get(Self::ordx_block_tx_outputs_inscriptions),
                )
                .layer(Extension(index))
                .layer(Extension(server_config.clone()))
                .layer(Extension(settings.clone()))
                .layer(SetResponseHeaderLayer::if_not_present(
                    header::CONTENT_SECURITY_POLICY,
                    HeaderValue::from_static("default-src 'self'"),
                ))
                .layer(SetResponseHeaderLayer::overriding(
                    header::STRICT_TRANSPORT_SECURITY,
                    HeaderValue::from_static("max-age=31536000; includeSubDomains; preload"),
                ))
                .layer(
                    CorsLayer::new()
                        .allow_methods([http::Method::GET])
                        .allow_origin(Any),
                )
                .layer(CompressionLayer::new())
                .with_state(server_config);

            let router = if let Some((username, password)) = settings.credentials() {
                router.layer(ValidateRequestHeaderLayer::basic(username, password))
            } else {
                router
            };

            match (self.http_port(), self.https_port()) {
                (Some(http_port), None) => {
                    self.spawn(router, handle, http_port, SpawnConfig::Http)?
                        .await??
                }
                (None, Some(https_port)) => {
                    self.spawn(
                        router,
                        handle,
                        https_port,
                        SpawnConfig::Https(self.acceptor(&settings)?),
                    )?
                    .await??
                }
                (Some(http_port), Some(https_port)) => {
                    let http_spawn_config = if self.redirect_http_to_https {
                        SpawnConfig::Redirect(if https_port == 443 {
                            format!("https://{}", acme_domains[0])
                        } else {
                            format!("https://{}:{https_port}", acme_domains[0])
                        })
                    } else {
                        SpawnConfig::Http
                    };

                    let (http_result, https_result) = tokio::join!(
                        self.spawn(router.clone(), handle.clone(), http_port, http_spawn_config)?,
                        self.spawn(
                            router,
                            handle,
                            https_port,
                            SpawnConfig::Https(self.acceptor(&settings)?),
                        )?
                    );
                    http_result.and(https_result)??;
                }
                (None, None) => unreachable!(),
            }

            Ok(None)
        })
    }

    fn spawn(
        &self,
        router: Router,
        handle: Handle,
        port: u16,
        config: SpawnConfig,
    ) -> Result<task::JoinHandle<io::Result<()>>> {
        let address = match &self.address {
            Some(address) => address.as_str(),
            None => {
                if cfg!(test) {
                    "127.0.0.1"
                } else {
                    "0.0.0.0"
                }
            }
        };

        let addr = (address, port)
            .to_socket_addrs()?
            .next()
            .ok_or_else(|| anyhow!("failed to get socket addrs"))?;

        if !cfg!(test) {
            eprintln!(
                "Listening on {}://{addr}",
                match config {
                    SpawnConfig::Https(_) => "https",
                    _ => "http",
                }
            );
        }

        Ok(tokio::spawn(async move {
            match config {
                SpawnConfig::Https(acceptor) => {
                    axum_server::Server::bind(addr)
                        .handle(handle)
                        .acceptor(acceptor)
                        .serve(router.into_make_service())
                        .await
                }
                SpawnConfig::Redirect(destination) => {
                    axum_server::Server::bind(addr)
                        .handle(handle)
                        .serve(
                            Router::new()
                                .fallback(Self::redirect_http_to_https)
                                .layer(Extension(destination))
                                .into_make_service(),
                        )
                        .await
                }
                SpawnConfig::Http => {
                    axum_server::Server::bind(addr)
                        .handle(handle)
                        .serve(router.into_make_service())
                        .await
                }
            }
        }))
    }

    fn acme_cache(acme_cache: Option<&PathBuf>, settings: &Settings) -> PathBuf {
        match acme_cache {
            Some(acme_cache) => acme_cache.clone(),
            None => settings.data_dir().join("acme-cache"),
        }
    }

    fn acme_domains(&self) -> Result<Vec<String>> {
        if !self.acme_domain.is_empty() {
            Ok(self.acme_domain.clone())
        } else {
            Ok(vec![
                System::host_name().ok_or(anyhow!("no hostname found"))?
            ])
        }
    }

    fn http_port(&self) -> Option<u16> {
        if self.http || self.http_port.is_some() || (self.https_port.is_none() && !self.https) {
            Some(self.http_port.unwrap_or(80))
        } else {
            None
        }
    }

    fn https_port(&self) -> Option<u16> {
        if self.https || self.https_port.is_some() {
            Some(self.https_port.unwrap_or(443))
        } else {
            None
        }
    }

    fn acceptor(&self, settings: &Settings) -> Result<AxumAcceptor> {
        let config = AcmeConfig::new(self.acme_domains()?)
            .contact(&self.acme_contact)
            .cache_option(Some(DirCache::new(Self::acme_cache(
                self.acme_cache.as_ref(),
                settings,
            ))))
            .directory(if cfg!(test) {
                LETS_ENCRYPT_STAGING_DIRECTORY
            } else {
                LETS_ENCRYPT_PRODUCTION_DIRECTORY
            });

        let mut state = config.state();

        let mut server_config = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_cert_resolver(state.resolver());

        server_config.alpn_protocols = vec!["h2".into(), "http/1.1".into()];

        let acceptor = state.axum_acceptor(Arc::new(server_config));

        tokio::spawn(async move {
            while let Some(result) = state.next().await {
                match result {
                    Ok(ok) => log::info!("ACME event: {:?}", ok),
                    Err(err) => log::error!("ACME error: {:?}", err),
                }
            }
        });

        Ok(acceptor)
    }

    fn index_height(index: &Index) -> ServerResult<Height> {
        index.block_height()?.ok_or_not_found(|| "genesis block")
    }

    async fn clock(Extension(index): Extension<Arc<Index>>) -> ServerResult<Response> {
        task::block_in_place(|| {
            Ok((
                [(
                    header::CONTENT_SECURITY_POLICY,
                    HeaderValue::from_static("default-src 'unsafe-inline'"),
                )],
                ClockSvg::new(Self::index_height(&index)?),
            )
                .into_response())
        })
    }

    async fn sat(
        Extension(server_config): Extension<Arc<ServerConfig>>,
        Extension(index): Extension<Arc<Index>>,
        Path(DeserializeFromStr(sat)): Path<DeserializeFromStr<Sat>>,
        AcceptJson(accept_json): AcceptJson,
    ) -> ServerResult<Response> {
        task::block_in_place(|| {
            let inscriptions = index.get_inscription_ids_by_sat(sat)?;
            let satpoint = index.rare_sat_satpoint(sat)?.or_else(|| {
                inscriptions.first().and_then(|&first_inscription_id| {
                    index
                        .get_inscription_satpoint_by_id(first_inscription_id)
                        .ok()
                        .flatten()
                })
            });
            let blocktime = index.block_time(sat.height())?;
            Ok(if accept_json {
                Json(api::Sat {
                    number: sat.0,
                    decimal: sat.decimal().to_string(),
                    degree: sat.degree().to_string(),
                    name: sat.name(),
                    block: sat.height().0,
                    cycle: sat.cycle(),
                    epoch: sat.epoch().0,
                    period: sat.period(),
                    offset: sat.third(),
                    rarity: sat.rarity(),
                    percentile: sat.percentile(),
                    satpoint,
                    timestamp: blocktime.timestamp().timestamp(),
                    inscriptions,
                })
                .into_response()
            } else {
                SatHtml {
                    sat,
                    satpoint,
                    blocktime,
                    inscriptions,
                }
                .page(server_config)
                .into_response()
            })
        })
    }

    async fn ordinal(Path(sat): Path<String>) -> Redirect {
        Redirect::to(&format!("/sat/{sat}"))
    }

    async fn output(
        Extension(server_config): Extension<Arc<ServerConfig>>,
        Extension(index): Extension<Arc<Index>>,
        Path(outpoint): Path<OutPoint>,
        AcceptJson(accept_json): AcceptJson,
    ) -> ServerResult<Response> {
        task::block_in_place(|| {
            let sat_ranges = index.list(outpoint)?;

            let indexed;

            let output = if outpoint == OutPoint::null() || outpoint == unbound_outpoint() {
                let mut value = 0;

                if let Some(ranges) = &sat_ranges {
                    for (start, end) in ranges {
                        value += end - start;
                    }
                }

                indexed = true;

                TxOut {
                    value,
                    script_pubkey: ScriptBuf::new(),
                }
            } else {
                indexed = index.contains_output(&outpoint)?;

                index
                    .get_transaction(outpoint.txid)?
                    .ok_or_not_found(|| format!("output {outpoint}"))?
                    .output
                    .into_iter()
                    .nth(outpoint.vout as usize)
                    .ok_or_not_found(|| format!("output {outpoint}"))?
            };

            let inscriptions = index.get_inscriptions_on_output(outpoint)?;

            let runes = index.get_rune_balances_for_outpoint(outpoint)?;

            let spent = index.is_output_spent(outpoint)?;

            Ok(if accept_json {
                Json(api::Output::new(
                    server_config.chain,
                    inscriptions,
                    outpoint,
                    output,
                    indexed,
                    runes,
                    sat_ranges,
                    spent,
                ))
                .into_response()
            } else {
                OutputHtml {
                    chain: server_config.chain,
                    inscriptions,
                    outpoint,
                    output,
                    runes,
                    sat_ranges,
                    spent,
                }
                .page(server_config)
                .into_response()
            })
        })
    }

    async fn range(
        Extension(server_config): Extension<Arc<ServerConfig>>,
        Path((DeserializeFromStr(start), DeserializeFromStr(end))): Path<(
            DeserializeFromStr<Sat>,
            DeserializeFromStr<Sat>,
        )>,
    ) -> ServerResult<PageHtml<RangeHtml>> {
        match start.cmp(&end) {
            Ordering::Equal => Err(ServerError::BadRequest("empty range".to_string())),
            Ordering::Greater => Err(ServerError::BadRequest(
                "range start greater than range end".to_string(),
            )),
            Ordering::Less => Ok(RangeHtml { start, end }.page(server_config)),
        }
    }

    async fn rare_txt(Extension(index): Extension<Arc<Index>>) -> ServerResult<RareTxt> {
        task::block_in_place(|| Ok(RareTxt(index.rare_sat_satpoints()?)))
    }

    async fn rune(
        Extension(server_config): Extension<Arc<ServerConfig>>,
        Extension(index): Extension<Arc<Index>>,
        Path(DeserializeFromStr(rune_query)): Path<DeserializeFromStr<query::Rune>>,
        AcceptJson(accept_json): AcceptJson,
    ) -> ServerResult<Response> {
        task::block_in_place(|| {
            if !index.has_rune_index() {
                return Err(ServerError::NotFound(
                    "this server has no rune index".to_string(),
                ));
            }

            let rune = match rune_query {
                query::Rune::SpacedRune(spaced_rune) => spaced_rune.rune,
                query::Rune::RuneId(rune_id) => index
                    .get_rune_by_id(rune_id)?
                    .ok_or_not_found(|| format!("rune {rune_id}"))?,
            };

            let (id, entry, parent) = index
                .rune(rune)?
                .ok_or_not_found(|| format!("rune {rune}"))?;

            Ok(if accept_json {
                Json(api::Rune { entry, id, parent }).into_response()
            } else {
                RuneHtml { entry, id, parent }
                    .page(server_config)
                    .into_response()
            })
        })
    }

    async fn runes(
        Extension(server_config): Extension<Arc<ServerConfig>>,
        Extension(index): Extension<Arc<Index>>,
        AcceptJson(accept_json): AcceptJson,
    ) -> ServerResult<Response> {
        task::block_in_place(|| {
            Ok(if accept_json {
                Json(api::Runes {
                    entries: index.runes()?,
                })
                .into_response()
            } else {
                RunesHtml {
                    entries: index.runes()?,
                }
                .page(server_config)
                .into_response()
            })
        })
    }

    async fn runes_balances(
        Extension(server_config): Extension<Arc<ServerConfig>>,
        Extension(index): Extension<Arc<Index>>,
        AcceptJson(accept_json): AcceptJson,
    ) -> ServerResult<Response> {
        task::block_in_place(|| {
            let balances = index.get_rune_balance_map()?;
            Ok(if accept_json {
                Json(balances).into_response()
            } else {
                RuneBalancesHtml { balances }
                    .page(server_config)
                    .into_response()
            })
        })
    }

    async fn home(
        Extension(server_config): Extension<Arc<ServerConfig>>,
        Extension(index): Extension<Arc<Index>>,
    ) -> ServerResult<PageHtml<HomeHtml>> {
        task::block_in_place(|| {
            Ok(HomeHtml {
                inscriptions: index.get_home_inscriptions()?,
            }
            .page(server_config))
        })
    }

    async fn blocks(
        Extension(server_config): Extension<Arc<ServerConfig>>,
        Extension(index): Extension<Arc<Index>>,
        AcceptJson(accept_json): AcceptJson,
    ) -> ServerResult<Response> {
        task::block_in_place(|| {
            let blocks = index.blocks(100)?;
            let mut featured_blocks = BTreeMap::new();
            for (height, hash) in blocks.iter().take(5) {
                let (inscriptions, _total_num) =
                    index.get_highest_paying_inscriptions_in_block(*height, 8)?;

                featured_blocks.insert(*hash, inscriptions);
            }

            Ok(if accept_json {
                Json(api::Blocks::new(blocks, featured_blocks)).into_response()
            } else {
                BlocksHtml::new(blocks, featured_blocks)
                    .page(server_config)
                    .into_response()
            })
        })
    }

    async fn install_script() -> Redirect {
        Redirect::to("https://raw.githubusercontent.com/ordinals/ord/master/install.sh")
    }

    async fn block(
        Extension(server_config): Extension<Arc<ServerConfig>>,
        Extension(index): Extension<Arc<Index>>,
        Path(DeserializeFromStr(query)): Path<DeserializeFromStr<query::Block>>,
        AcceptJson(accept_json): AcceptJson,
    ) -> ServerResult<Response> {
        task::block_in_place(|| {
            let (block, height) = match query {
                query::Block::Height(height) => {
                    let block = index
                        .get_block_by_height(height)?
                        .ok_or_not_found(|| format!("block {height}"))?;

                    (block, height)
                }
                query::Block::Hash(hash) => {
                    let info = index
                        .block_header_info(hash)?
                        .ok_or_not_found(|| format!("block {hash}"))?;

                    let block = index
                        .get_block_by_hash(hash)?
                        .ok_or_not_found(|| format!("block {hash}"))?;

                    (block, u32::try_from(info.height).unwrap())
                }
            };

            Ok(if accept_json {
                let inscriptions = index.get_inscriptions_in_block(height)?;
                Json(api::Block::new(
                    block,
                    Height(height),
                    Self::index_height(&index)?,
                    inscriptions,
                ))
                .into_response()
            } else {
                let (featured_inscriptions, total_num) =
                    index.get_highest_paying_inscriptions_in_block(height, 8)?;
                BlockHtml::new(
                    block,
                    Height(height),
                    Self::index_height(&index)?,
                    total_num,
                    featured_inscriptions,
                )
                .page(server_config)
                .into_response()
            })
        })
    }

    async fn transaction(
        Extension(server_config): Extension<Arc<ServerConfig>>,
        Extension(index): Extension<Arc<Index>>,
        Path(txid): Path<Txid>,
        AcceptJson(accept_json): AcceptJson,
    ) -> ServerResult<Response> {
        task::block_in_place(|| {
            let transaction = index
                .get_transaction(txid)?
                .ok_or_not_found(|| format!("transaction {txid}"))?;

            let inscription_count = index.inscription_count(txid)?;

            Ok(if accept_json {
                Json(api::Transaction {
                    chain: server_config.chain,
                    etching: index.get_etching(txid)?,
                    inscription_count,
                    transaction,
                    txid,
                })
                .into_response()
            } else {
                TransactionHtml {
                    chain: server_config.chain,
                    etching: index.get_etching(txid)?,
                    inscription_count,
                    transaction,
                    txid,
                }
                .page(server_config)
                .into_response()
            })
        })
    }

    async fn metadata(
        Extension(index): Extension<Arc<Index>>,
        Path(inscription_id): Path<InscriptionId>,
    ) -> ServerResult<Json<String>> {
        task::block_in_place(|| {
            let metadata = index
                .get_inscription_by_id(inscription_id)?
                .ok_or_not_found(|| format!("inscription {inscription_id}"))?
                .metadata
                .ok_or_not_found(|| format!("inscription {inscription_id} metadata"))?;

            Ok(Json(hex::encode(metadata)))
        })
    }

    async fn inscription_recursive(
        Extension(index): Extension<Arc<Index>>,
        Path(inscription_id): Path<InscriptionId>,
    ) -> ServerResult<Response> {
        task::block_in_place(|| {
            let inscription = index
                .get_inscription_by_id(inscription_id)?
                .ok_or_not_found(|| format!("inscription {inscription_id}"))?;

            let entry = index
                .get_inscription_entry(inscription_id)
                .unwrap()
                .unwrap();

            let satpoint = index
                .get_inscription_satpoint_by_id(inscription_id)
                .ok()
                .flatten()
                .unwrap();

            let output = if satpoint.outpoint == unbound_outpoint() {
                None
            } else {
                Some(
                    index
                        .get_transaction(satpoint.outpoint.txid)?
                        .ok_or_not_found(|| {
                            format!("inscription {inscription_id} current transaction")
                        })?
                        .output
                        .into_iter()
                        .nth(satpoint.outpoint.vout.try_into().unwrap())
                        .ok_or_not_found(|| {
                            format!("inscription {inscription_id} current transaction output")
                        })?,
                )
            };

            Ok(Json(api::InscriptionRecursive {
                charms: Charm::ALL
                    .iter()
                    .filter(|charm| charm.is_set(entry.charms))
                    .map(|charm| charm.title().into())
                    .collect(),
                content_type: inscription.content_type().map(|s| s.to_string()),
                content_length: inscription.content_length(),
                fee: entry.fee,
                height: entry.height,
                id: inscription_id,
                number: entry.inscription_number,
                output: satpoint.outpoint,
                value: output.as_ref().map(|o| o.value),
                sat: entry.sat,
                satpoint,
                timestamp: timestamp(entry.timestamp).timestamp(),
            })
            .into_response())
        })
    }

    async fn status(
        Extension(server_config): Extension<Arc<ServerConfig>>,
        Extension(index): Extension<Arc<Index>>,
        AcceptJson(accept_json): AcceptJson,
    ) -> ServerResult<Response> {
        task::block_in_place(|| {
            Ok(if accept_json {
                Json(index.status()?).into_response()
            } else {
                index.status()?.page(server_config).into_response()
            })
        })
    }

    async fn search_by_query(
        Extension(index): Extension<Arc<Index>>,
        Query(search): Query<Search>,
    ) -> ServerResult<Redirect> {
        Self::search(index, search.query).await
    }

    async fn search_by_path(
        Extension(index): Extension<Arc<Index>>,
        Path(search): Path<Search>,
    ) -> ServerResult<Redirect> {
        Self::search(index, search.query).await
    }

    async fn search(index: Arc<Index>, query: String) -> ServerResult<Redirect> {
        Self::search_inner(index, query).await
    }

    async fn search_inner(index: Arc<Index>, query: String) -> ServerResult<Redirect> {
        task::block_in_place(|| {
            lazy_static! {
                static ref HASH: Regex = Regex::new(r"^[[:xdigit:]]{64}$").unwrap();
                static ref INSCRIPTION_ID: Regex = Regex::new(r"^[[:xdigit:]]{64}i\d+$").unwrap();
                static ref OUTPOINT: Regex = Regex::new(r"^[[:xdigit:]]{64}:\d+$").unwrap();
                static ref RUNE: Regex = Regex::new(r"^[A-Z•.]+$").unwrap();
                static ref RUNE_ID: Regex = Regex::new(r"^[0-9]+:[0-9]+$").unwrap();
            }

            let query = query.trim();

            if HASH.is_match(query) {
                if index.block_header(query.parse().unwrap())?.is_some() {
                    Ok(Redirect::to(&format!("/block/{query}")))
                } else {
                    Ok(Redirect::to(&format!("/tx/{query}")))
                }
            } else if OUTPOINT.is_match(query) {
                Ok(Redirect::to(&format!("/output/{query}")))
            } else if INSCRIPTION_ID.is_match(query) {
                Ok(Redirect::to(&format!("/inscription/{query}")))
            } else if RUNE.is_match(query) {
                Ok(Redirect::to(&format!("/rune/{query}")))
            } else if RUNE_ID.is_match(query) {
                let id = query
                    .parse::<RuneId>()
                    .map_err(|err| ServerError::BadRequest(err.to_string()))?;

                let rune = index.get_rune_by_id(id)?.ok_or_not_found(|| "rune ID")?;

                Ok(Redirect::to(&format!("/rune/{rune}")))
            } else {
                Ok(Redirect::to(&format!("/sat/{query}")))
            }
        })
    }

    async fn favicon() -> ServerResult<Response> {
        Ok(Self::static_asset(Path("/favicon.png".to_string()))
            .await
            .into_response())
    }

    async fn feed(
        Extension(server_config): Extension<Arc<ServerConfig>>,
        Extension(index): Extension<Arc<Index>>,
    ) -> ServerResult<Response> {
        task::block_in_place(|| {
            let mut builder = rss::ChannelBuilder::default();

            let chain = server_config.chain;
            match chain {
                Chain::Mainnet => builder.title("Inscriptions".to_string()),
                _ => builder.title(format!("Inscriptions – {chain:?}")),
            };

            builder.generator(Some("ord".to_string()));

            for (number, id) in index.get_feed_inscriptions(300)? {
                builder.item(
                    rss::ItemBuilder::default()
                        .title(Some(format!("Inscription {number}")))
                        .link(Some(format!("/inscription/{id}")))
                        .guid(Some(rss::Guid {
                            value: format!("/inscription/{id}"),
                            permalink: true,
                        }))
                        .build(),
                );
            }

            Ok((
                [
                    (header::CONTENT_TYPE, "application/rss+xml"),
                    (
                        header::CONTENT_SECURITY_POLICY,
                        "default-src 'unsafe-inline'",
                    ),
                ],
                builder.build().to_string(),
            )
                .into_response())
        })
    }

    async fn static_asset(Path(path): Path<String>) -> ServerResult<Response> {
        let content = StaticAssets::get(if let Some(stripped) = path.strip_prefix('/') {
            stripped
        } else {
            &path
        })
        .ok_or_not_found(|| format!("asset {path}"))?;
        let body = body::boxed(body::Full::from(content.data));
        let mime = mime_guess::from_path(path).first_or_octet_stream();
        Ok(Response::builder()
            .header(header::CONTENT_TYPE, mime.as_ref())
            .body(body)
            .unwrap())
    }

    async fn block_count(Extension(index): Extension<Arc<Index>>) -> ServerResult<String> {
        task::block_in_place(|| Ok(index.block_count()?.to_string()))
    }

    async fn block_height(Extension(index): Extension<Arc<Index>>) -> ServerResult<String> {
        task::block_in_place(|| {
            Ok(index
                .block_height()?
                .ok_or_not_found(|| "blockheight")?
                .to_string())
        })
    }

    async fn block_hash(Extension(index): Extension<Arc<Index>>) -> ServerResult<String> {
        task::block_in_place(|| {
            Ok(index
                .block_hash(None)?
                .ok_or_not_found(|| "blockhash")?
                .to_string())
        })
    }

    async fn block_hash_json(
        Extension(index): Extension<Arc<Index>>,
    ) -> ServerResult<Json<String>> {
        task::block_in_place(|| {
            Ok(Json(
                index
                    .block_hash(None)?
                    .ok_or_not_found(|| "blockhash")?
                    .to_string(),
            ))
        })
    }

    async fn block_hash_from_height(
        Extension(index): Extension<Arc<Index>>,
        Path(height): Path<u32>,
    ) -> ServerResult<String> {
        task::block_in_place(|| {
            Ok(index
                .block_hash(Some(height))?
                .ok_or_not_found(|| "blockhash")?
                .to_string())
        })
    }

    async fn block_hash_from_height_json(
        Extension(index): Extension<Arc<Index>>,
        Path(height): Path<u32>,
    ) -> ServerResult<Json<String>> {
        task::block_in_place(|| {
            Ok(Json(
                index
                    .block_hash(Some(height))?
                    .ok_or_not_found(|| "blockhash")?
                    .to_string(),
            ))
        })
    }

    async fn block_info(
        Extension(index): Extension<Arc<Index>>,
        Path(DeserializeFromStr(query)): Path<DeserializeFromStr<query::Block>>,
    ) -> ServerResult<Json<api::BlockInfo>> {
        task::block_in_place(|| {
            let hash = match query {
                query::Block::Hash(hash) => hash,
                query::Block::Height(height) => index
                    .block_hash(Some(height))?
                    .ok_or_not_found(|| format!("block {height}"))?,
            };

            let header = index
                .block_header(hash)?
                .ok_or_not_found(|| format!("block {hash}"))?;

            let info = index
                .block_header_info(hash)?
                .ok_or_not_found(|| format!("block {hash}"))?;

            let stats = index
                .block_stats(info.height.try_into().unwrap())?
                .ok_or_not_found(|| format!("block {hash}"))?;

            Ok(Json(api::BlockInfo {
                average_fee: stats.avg_fee.to_sat(),
                average_fee_rate: stats.avg_fee_rate.to_sat(),
                bits: header.bits.to_consensus(),
                chainwork: info.chainwork.try_into().unwrap(),
                confirmations: info.confirmations,
                difficulty: info.difficulty,
                hash,
                height: info.height.try_into().unwrap(),
                max_fee: stats.max_fee.to_sat(),
                max_fee_rate: stats.max_fee_rate.to_sat(),
                max_tx_size: stats.max_tx_size,
                median_fee: stats.median_fee.to_sat(),
                median_time: info
                    .median_time
                    .map(|median_time| median_time.try_into().unwrap()),
                merkle_root: info.merkle_root,
                min_fee: stats.min_fee.to_sat(),
                min_fee_rate: stats.min_fee_rate.to_sat(),
                next_block: info.next_block_hash,
                nonce: info.nonce,
                previous_block: info.previous_block_hash,
                subsidy: stats.subsidy.to_sat(),
                target: target_as_block_hash(header.target()),
                timestamp: info.time.try_into().unwrap(),
                total_fee: stats.total_fee.to_sat(),
                total_size: stats.total_size,
                total_weight: stats.total_weight,
                transaction_count: info.n_tx.try_into().unwrap(),
                #[allow(clippy::cast_sign_loss)]
                version: info.version.to_consensus() as u32,
            }))
        })
    }

    async fn block_time(Extension(index): Extension<Arc<Index>>) -> ServerResult<String> {
        task::block_in_place(|| {
            Ok(index
                .block_time(index.block_height()?.ok_or_not_found(|| "blocktime")?)?
                .unix_timestamp()
                .to_string())
        })
    }

    async fn input(
        Extension(server_config): Extension<Arc<ServerConfig>>,
        Extension(index): Extension<Arc<Index>>,
        Path(path): Path<(u32, usize, usize)>,
    ) -> ServerResult<PageHtml<InputHtml>> {
        task::block_in_place(|| {
            let not_found = || format!("input /{}/{}/{}", path.0, path.1, path.2);

            let block = index
                .get_block_by_height(path.0)?
                .ok_or_not_found(not_found)?;

            let transaction = block
                .txdata
                .into_iter()
                .nth(path.1)
                .ok_or_not_found(not_found)?;

            let input = transaction
                .input
                .into_iter()
                .nth(path.2)
                .ok_or_not_found(not_found)?;

            Ok(InputHtml { path, input }.page(server_config))
        })
    }

    async fn faq() -> Redirect {
        Redirect::to("https://docs.ordinals.com/faq/")
    }

    async fn bounties() -> Redirect {
        Redirect::to("https://docs.ordinals.com/bounty/")
    }

    fn proxy_content(proxy: &Url, inscription_id: InscriptionId) -> ServerResult<Response> {
        let response = reqwest::blocking::Client::new()
            .get(format!("{}content/{}", proxy, inscription_id))
            .send()
            .map_err(|err| anyhow!(err))?;

        let mut headers = response.headers().clone();

        headers.insert(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_str(&format!(
                "default-src 'self' {proxy} 'unsafe-eval' 'unsafe-inline' data: blob:"
            ))
            .map_err(|err| ServerError::Internal(Error::from(err)))?,
        );

        Ok((
            response.status(),
            headers,
            response.bytes().map_err(|err| anyhow!(err))?,
        )
            .into_response())
    }

    async fn content(
        Extension(index): Extension<Arc<Index>>,
        Extension(settings): Extension<Arc<Settings>>,
        Extension(server_config): Extension<Arc<ServerConfig>>,
        Path(inscription_id): Path<InscriptionId>,
        accept_encoding: AcceptEncoding,
    ) -> ServerResult<Response> {
        task::block_in_place(|| {
            if settings.is_hidden(inscription_id) {
                return Ok(PreviewUnknownHtml.into_response());
            }

            let Some(mut inscription) = index.get_inscription_by_id(inscription_id)? else {
        return if let Some(proxy) = server_config.content_proxy.as_ref() {
          Self::proxy_content(proxy, inscription_id)
        } else {
          Err(ServerError::NotFound(format!(
            "{} not found",
            inscription_id
          )))
        };
      };

            if let Some(delegate) = inscription.delegate() {
                inscription = index
                    .get_inscription_by_id(delegate)?
                    .ok_or_not_found(|| format!("delegate {inscription_id}"))?
            }

            Ok(
                Self::content_response(inscription, accept_encoding, &server_config)?
                    .ok_or_not_found(|| format!("inscription {inscription_id} content"))?
                    .into_response(),
            )
        })
    }

    fn content_response(
        inscription: Inscription,
        accept_encoding: AcceptEncoding,
        server_config: &ServerConfig,
    ) -> ServerResult<Option<(HeaderMap, Vec<u8>)>> {
        let mut headers = HeaderMap::new();

        match &server_config.csp_origin {
            None => {
                headers.insert(
                    header::CONTENT_SECURITY_POLICY,
                    HeaderValue::from_static(
                        "default-src 'self' 'unsafe-eval' 'unsafe-inline' data: blob:",
                    ),
                );
                headers.append(
          header::CONTENT_SECURITY_POLICY,
          HeaderValue::from_static("default-src *:*/content/ *:*/blockheight *:*/blockhash *:*/blockhash/ *:*/blocktime *:*/r/ 'unsafe-eval' 'unsafe-inline' data: blob:"),
        );
            }
            Some(origin) => {
                let csp = format!("default-src {origin}/content/ {origin}/blockheight {origin}/blockhash {origin}/blockhash/ {origin}/blocktime {origin}/r/ 'unsafe-eval' 'unsafe-inline' data: blob:");
                headers.insert(
                    header::CONTENT_SECURITY_POLICY,
                    HeaderValue::from_str(&csp)
                        .map_err(|err| ServerError::Internal(Error::from(err)))?,
                );
            }
        }

        headers.insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("public, max-age=1209600, immutable"),
        );

        headers.insert(
            header::CONTENT_TYPE,
            inscription
                .content_type()
                .and_then(|content_type| content_type.parse().ok())
                .unwrap_or(HeaderValue::from_static("application/octet-stream")),
        );

        if let Some(content_encoding) = inscription.content_encoding() {
            if accept_encoding.is_acceptable(&content_encoding) {
                headers.insert(header::CONTENT_ENCODING, content_encoding);
            } else if server_config.decompress && content_encoding == "br" {
                let Some(body) = inscription.into_body() else {
          return Ok(None);
        };

                let mut decompressed = Vec::new();

                Decompressor::new(body.as_slice(), 4096)
                    .read_to_end(&mut decompressed)
                    .map_err(|err| ServerError::Internal(err.into()))?;

                return Ok(Some((headers, decompressed)));
            } else {
                return Err(ServerError::NotAcceptable {
                    accept_encoding,
                    content_encoding,
                });
            }
        }

        let Some(body) = inscription.into_body() else {
      return Ok(None);
    };

        Ok(Some((headers, body)))
    }

    async fn preview(
        Extension(index): Extension<Arc<Index>>,
        Extension(settings): Extension<Arc<Settings>>,
        Extension(server_config): Extension<Arc<ServerConfig>>,
        Path(inscription_id): Path<InscriptionId>,
        accept_encoding: AcceptEncoding,
    ) -> ServerResult<Response> {
        task::block_in_place(|| {
            if settings.is_hidden(inscription_id) {
                return Ok(PreviewUnknownHtml.into_response());
            }

            let mut inscription = index
                .get_inscription_by_id(inscription_id)?
                .ok_or_not_found(|| format!("inscription {inscription_id}"))?;

            if let Some(delegate) = inscription.delegate() {
                inscription = index
                    .get_inscription_by_id(delegate)?
                    .ok_or_not_found(|| format!("delegate {inscription_id}"))?
            }

            match inscription.media() {
                Media::Audio => Ok(PreviewAudioHtml { inscription_id }.into_response()),
                Media::Code(language) => Ok((
                    [(
                        header::CONTENT_SECURITY_POLICY,
                        "script-src-elem 'self' https://cdn.jsdelivr.net",
                    )],
                    PreviewCodeHtml {
                        inscription_id,
                        language,
                    },
                )
                    .into_response()),
                Media::Font => Ok((
                    [(
                        header::CONTENT_SECURITY_POLICY,
                        "script-src-elem 'self'; style-src 'self' 'unsafe-inline';",
                    )],
                    PreviewFontHtml { inscription_id },
                )
                    .into_response()),
                Media::Iframe => {
                    Ok(
                        Self::content_response(inscription, accept_encoding, &server_config)?
                            .ok_or_not_found(|| format!("inscription {inscription_id} content"))?
                            .into_response(),
                    )
                }
                Media::Image(image_rendering) => Ok((
                    [(
                        header::CONTENT_SECURITY_POLICY,
                        "default-src 'self' 'unsafe-inline'",
                    )],
                    PreviewImageHtml {
                        image_rendering,
                        inscription_id,
                    },
                )
                    .into_response()),
                Media::Markdown => Ok((
                    [(
                        header::CONTENT_SECURITY_POLICY,
                        "script-src-elem 'self' https://cdn.jsdelivr.net",
                    )],
                    PreviewMarkdownHtml { inscription_id },
                )
                    .into_response()),
                Media::Model => Ok((
                    [(
                        header::CONTENT_SECURITY_POLICY,
                        "script-src-elem 'self' https://ajax.googleapis.com",
                    )],
                    PreviewModelHtml { inscription_id },
                )
                    .into_response()),
                Media::Pdf => Ok((
                    [(
                        header::CONTENT_SECURITY_POLICY,
                        "script-src-elem 'self' https://cdn.jsdelivr.net",
                    )],
                    PreviewPdfHtml { inscription_id },
                )
                    .into_response()),
                Media::Text => Ok(PreviewTextHtml { inscription_id }.into_response()),
                Media::Unknown => Ok(PreviewUnknownHtml.into_response()),
                Media::Video => Ok(PreviewVideoHtml { inscription_id }.into_response()),
            }
        })
    }

    async fn inscription(
        Extension(server_config): Extension<Arc<ServerConfig>>,
        Extension(index): Extension<Arc<Index>>,
        Path(DeserializeFromStr(query)): Path<DeserializeFromStr<query::Inscription>>,
        AcceptJson(accept_json): AcceptJson,
    ) -> ServerResult<Response> {
        task::block_in_place(|| {
            let info = Index::inscription_info(&index, query)?
                .ok_or_not_found(|| format!("inscription {query}"))?;

            Ok(if accept_json {
                Json(api::Inscription {
                    address: info
                        .output
                        .as_ref()
                        .and_then(|o| {
                            server_config
                                .chain
                                .address_from_script(&o.script_pubkey)
                                .ok()
                        })
                        .map(|address| address.to_string()),
                    charms: Charm::ALL
                        .iter()
                        .filter(|charm| charm.is_set(info.charms))
                        .map(|charm| charm.title().into())
                        .collect(),
                    children: info.children,
                    content_length: info.inscription.content_length(),
                    content_type: info.inscription.content_type().map(|s| s.to_string()),
                    fee: info.entry.fee,
                    height: info.entry.height,
                    id: info.entry.id,
                    next: info.next,
                    number: info.entry.inscription_number,
                    parent: info.parent,
                    previous: info.previous,
                    rune: info.rune,
                    sat: info.entry.sat,
                    satpoint: info.satpoint,
                    timestamp: timestamp(info.entry.timestamp).timestamp(),
                    value: info.output.as_ref().map(|o| o.value),
                })
                .into_response()
            } else {
                InscriptionHtml {
                    chain: server_config.chain,
                    charms: Charm::Vindicated.unset(info.charms),
                    children: info.children,
                    fee: info.entry.fee,
                    height: info.entry.height,
                    inscription: info.inscription,
                    id: info.entry.id,
                    number: info.entry.inscription_number,
                    next: info.next,
                    output: info.output,
                    parent: info.parent,
                    previous: info.previous,
                    rune: info.rune,
                    sat: info.entry.sat,
                    satpoint: info.satpoint,
                    timestamp: timestamp(info.entry.timestamp),
                }
                .page(server_config)
                .into_response()
            })
        })
    }

    async fn collections(
        Extension(server_config): Extension<Arc<ServerConfig>>,
        Extension(index): Extension<Arc<Index>>,
    ) -> ServerResult<Response> {
        Self::collections_paginated(Extension(server_config), Extension(index), Path(0)).await
    }

    async fn collections_paginated(
        Extension(server_config): Extension<Arc<ServerConfig>>,
        Extension(index): Extension<Arc<Index>>,
        Path(page_index): Path<usize>,
    ) -> ServerResult<Response> {
        task::block_in_place(|| {
            let (collections, more_collections) =
                index.get_collections_paginated(100, page_index)?;

            let prev = page_index.checked_sub(1);

            let next = more_collections.then_some(page_index + 1);

            Ok(CollectionsHtml {
                inscriptions: collections,
                prev,
                next,
            }
            .page(server_config)
            .into_response())
        })
    }

    async fn children(
        Extension(server_config): Extension<Arc<ServerConfig>>,
        Extension(index): Extension<Arc<Index>>,
        Path(inscription_id): Path<InscriptionId>,
    ) -> ServerResult<Response> {
        Self::children_paginated(
            Extension(server_config),
            Extension(index),
            Path((inscription_id, 0)),
        )
        .await
    }

    async fn children_paginated(
        Extension(server_config): Extension<Arc<ServerConfig>>,
        Extension(index): Extension<Arc<Index>>,
        Path((parent, page)): Path<(InscriptionId, usize)>,
    ) -> ServerResult<Response> {
        task::block_in_place(|| {
            let entry = index
                .get_inscription_entry(parent)?
                .ok_or_not_found(|| format!("inscription {parent}"))?;

            let parent_number = entry.inscription_number;

            let (children, more_children) = index.get_children_by_sequence_number_paginated(
                entry.sequence_number,
                100,
                page,
            )?;

            let prev_page = page.checked_sub(1);

            let next_page = more_children.then_some(page + 1);

            Ok(ChildrenHtml {
                parent,
                parent_number,
                children,
                prev_page,
                next_page,
            }
            .page(server_config)
            .into_response())
        })
    }

    async fn children_recursive(
        Extension(index): Extension<Arc<Index>>,
        Path(inscription_id): Path<InscriptionId>,
    ) -> ServerResult<Response> {
        Self::children_recursive_paginated(Extension(index), Path((inscription_id, 0))).await
    }

    async fn children_recursive_paginated(
        Extension(index): Extension<Arc<Index>>,
        Path((parent, page)): Path<(InscriptionId, usize)>,
    ) -> ServerResult<Response> {
        task::block_in_place(|| {
            let parent_sequence_number = index
                .get_inscription_entry(parent)?
                .ok_or_not_found(|| format!("inscription {parent}"))?
                .sequence_number;

            let (ids, more) = index.get_children_by_sequence_number_paginated(
                parent_sequence_number,
                100,
                page,
            )?;

            Ok(Json(api::Children { ids, more, page }).into_response())
        })
    }

    async fn inscriptions(
        Extension(server_config): Extension<Arc<ServerConfig>>,
        Extension(index): Extension<Arc<Index>>,
        accept_json: AcceptJson,
    ) -> ServerResult<Response> {
        Self::inscriptions_paginated(
            Extension(server_config),
            Extension(index),
            Path(0),
            accept_json,
        )
        .await
    }

    async fn inscriptions_paginated(
        Extension(server_config): Extension<Arc<ServerConfig>>,
        Extension(index): Extension<Arc<Index>>,
        Path(page_index): Path<u32>,
        AcceptJson(accept_json): AcceptJson,
    ) -> ServerResult<Response> {
        task::block_in_place(|| {
            let (inscriptions, more) = index.get_inscriptions_paginated(100, page_index)?;

            let prev = page_index.checked_sub(1);

            let next = more.then_some(page_index + 1);

            Ok(if accept_json {
                Json(api::Inscriptions {
                    ids: inscriptions,
                    page_index,
                    more,
                })
                .into_response()
            } else {
                InscriptionsHtml {
                    inscriptions,
                    next,
                    prev,
                }
                .page(server_config)
                .into_response()
            })
        })
    }

    async fn inscriptions_in_block(
        Extension(server_config): Extension<Arc<ServerConfig>>,
        Extension(index): Extension<Arc<Index>>,
        Path(block_height): Path<u32>,
        AcceptJson(accept_json): AcceptJson,
    ) -> ServerResult<Response> {
        Self::inscriptions_in_block_paginated(
            Extension(server_config),
            Extension(index),
            Path((block_height, 0)),
            AcceptJson(accept_json),
        )
        .await
    }

    async fn inscriptions_in_block_paginated(
        Extension(server_config): Extension<Arc<ServerConfig>>,
        Extension(index): Extension<Arc<Index>>,
        Path((block_height, page_index)): Path<(u32, u32)>,
        AcceptJson(accept_json): AcceptJson,
    ) -> ServerResult<Response> {
        task::block_in_place(|| {
            let page_size = 100;

            let page_index_usize = usize::try_from(page_index).unwrap_or(usize::MAX);
            let page_size_usize = usize::try_from(page_size).unwrap_or(usize::MAX);

            let mut inscriptions = index
                .get_inscriptions_in_block(block_height)?
                .into_iter()
                .skip(page_index_usize.saturating_mul(page_size_usize))
                .take(page_size_usize.saturating_add(1))
                .collect::<Vec<InscriptionId>>();

            let more = inscriptions.len() > page_size_usize;

            if more {
                inscriptions.pop();
            }

            Ok(if accept_json {
                Json(api::Inscriptions {
                    ids: inscriptions,
                    page_index,
                    more,
                })
                .into_response()
            } else {
                InscriptionsBlockHtml::new(
                    block_height,
                    index.block_height()?.unwrap_or(Height(0)).n(),
                    inscriptions,
                    more,
                    page_index,
                )?
                .page(server_config)
                .into_response()
            })
        })
    }

    async fn sat_inscriptions(
        Extension(index): Extension<Arc<Index>>,
        Path(sat): Path<u64>,
    ) -> ServerResult<Json<api::SatInscriptions>> {
        Self::sat_inscriptions_paginated(Extension(index), Path((sat, 0))).await
    }

    async fn sat_inscriptions_paginated(
        Extension(index): Extension<Arc<Index>>,
        Path((sat, page)): Path<(u64, u64)>,
    ) -> ServerResult<Json<api::SatInscriptions>> {
        task::block_in_place(|| {
            if !index.has_sat_index() {
                return Err(ServerError::NotFound(
                    "this server has no sat index".to_string(),
                ));
            }

            let (ids, more) = index.get_inscription_ids_by_sat_paginated(Sat(sat), 100, page)?;

            Ok(Json(api::SatInscriptions { ids, more, page }))
        })
    }

    async fn sat_inscription_at_index(
        Extension(index): Extension<Arc<Index>>,
        Path((DeserializeFromStr(sat), inscription_index)): Path<(DeserializeFromStr<Sat>, isize)>,
    ) -> ServerResult<Json<api::SatInscription>> {
        task::block_in_place(|| {
            if !index.has_sat_index() {
                return Err(ServerError::NotFound(
                    "this server has no sat index".to_string(),
                ));
            }

            let id = index.get_inscription_id_by_sat_indexed(sat, inscription_index)?;

            Ok(Json(api::SatInscription { id }))
        })
    }

    async fn redirect_http_to_https(
        Extension(mut destination): Extension<String>,
        uri: Uri,
    ) -> Redirect {
        if let Some(path_and_query) = uri.path_and_query() {
            destination.push_str(path_and_query.as_str());
        }

        Redirect::to(&destination)
    }

    async fn ordx_block_inscriptions(
        Extension(server_config): Extension<Arc<ServerConfig>>,
        Extension(index): Extension<Arc<Index>>,
        Path(DeserializeFromStr(block_height)): Path<DeserializeFromStr<u32>>,
    ) -> ServerResult<Response> {
        Ok({
            let block = index
                .get_block_by_height(block_height)?
                .ok_or_not_found(|| format!("block {block_height}"))?;
            let inscription_id_list = index.get_inscriptions_in_block(block_height)?;
            let first_block_txid = match block.txdata.len() > 0 {
                true => block.txdata[0].txid().to_string(),
                false => Txid::all_zeros().to_string(),
            };
            log::info!("block-> height: {block_height:?} , firstBlockTxid: {first_block_txid:?}");
            // println!("block-> height: {block_height:?} firstBlockTxid: {first_block_txid:?}");
            Json(api::OrdxBlockInscriptions {
                height: block_height,
                inscriptions: inscription_id_list
                    .par_iter()
                    .map(|inscription_id| {
                        let query_inscription_id = query::Inscription::Id(*inscription_id);
                        let info = Index::inscription_info(&index, query_inscription_id)?
                            .ok_or_not_found(|| format!("inscription {query_inscription_id}"))?;

                        // api inscription
                        let ordx_inscription = api::OrdxInscription {
                            address: info
                                .output
                                .as_ref()
                                .and_then(|o| {
                                    server_config
                                        .chain
                                        .address_from_script(&o.script_pubkey)
                                        .ok()
                                })
                                .map(|address| address.to_string()),
                            // charms: Charm::ALL
                            //   .iter()
                            //   .filter(|charm| charm.is_set(info.charms))
                            //   .map(|charm| charm.title().into())
                            //   .collect(),
                            children: info.children,
                            content_length: info.inscription.content_length(),
                            content_type: info.inscription.content_type().map(|s| s.to_string()),
                            fee: info.entry.fee,
                            height: info.entry.height,
                            id: info.entry.id,
                            next: info.next,
                            number: info.entry.inscription_number,
                            parent: info.parent,
                            previous: info.previous,
                            // rune: info.rune,
                            sat: info.entry.sat,
                            satpoint: info.satpoint,
                            timestamp: timestamp(info.entry.timestamp).timestamp(),
                            value: info.output.as_ref().map(|o| o.value),
                        };

                        // api output
                        // let unbound_output = OutPoint {
                        //   txid: "0000000000000000000000000000000000000000000000000000000000000000"
                        //     .parse()
                        //     .unwrap(),
                        //   vout: 0,
                        // };
                        // let ordx_output = match ordx_inscription.satpoint.outpoint != unbound_output {
                        //   true => {
                        //     let outpoint = ordx_inscription.satpoint.outpoint;
                        //     // let sat_ranges = index.list(outpoint)?;
                        //     // let inscriptions = index.get_inscriptions_on_output(outpoint)?;
                        //     // let indexed = index.contains_output(&outpoint)?;
                        //     // let runes = index.get_rune_balances_for_outpoint(outpoint)?;
                        //     // let spent = index.is_output_spent(outpoint)?;
                        //     let output = index
                        //       .get_transaction(outpoint.txid)?
                        //       .ok_or_not_found(|| format!("output {outpoint}"))?
                        //       .output
                        //       .into_iter()
                        //       .nth(outpoint.vout as usize)
                        //       .ok_or_not_found(|| format!("output {outpoint}"))?;
                        //     Some(api::OrdxOutput::new(server_config.chain, outpoint, output))
                        //   }
                        //   false => None,
                        // };

                        // get geneses address from address
                        // When the output and inciption id are different, it means that the inscription has been traded, else this is first block tx
                        let mut outpoint = ordx_inscription.satpoint.outpoint;
                        if ordx_inscription.satpoint.outpoint.txid != inscription_id.txid
                            && ordx_inscription.satpoint.outpoint.txid.to_string()
                                != first_block_txid
                        {
                            let mut output_index = inscription_id.index;
                            let transaction = index
                                .get_transaction(inscription_id.txid)?
                                .ok_or_not_found(|| {
                                    format!("transaction {}", inscription_id.txid)
                                })?;
                            let output_len = transaction.output.len() as u32;
                            // cursed and blessed inscription share the same outpoint, ex: tx 219a5e5458bf0ba686f1c5660cf01652c88dec1b30c13571c43d97a9b11ac653
                            while output_index >= output_len {
                                output_index -= 1;
                            }
                            outpoint = OutPoint::new(inscription_id.txid, output_index)
                        }

                        // let sat_ranges = index.list(outpoint)?;
                        // let inscriptions = index.get_inscriptions_on_output(outpoint)?;
                        // let indexed = index.contains_output(&outpoint)?;
                        // let runes = index.get_rune_balances_for_outpoint(outpoint)?;
                        // let spent = index.is_output_spent(outpoint)?;
                        let output = index
                            .get_transaction(outpoint.txid)?
                            .ok_or_not_found(|| format!("output {outpoint}"))?
                            .output
                            .into_iter()
                            .nth(outpoint.vout as usize)
                            .ok_or_not_found(|| format!("output {outpoint}"))?;

                        // let api_geneses_output = api::Output::new(
                        //   server_config.chain,
                        //   inscriptions,
                        //   outpoint,
                        //   output,
                        //   indexed,
                        //   runes,
                        //   sat_ranges,
                        //   spent,
                        // );
                        let geneses_address = server_config
                            .chain
                            .address_from_script(&output.script_pubkey)
                            .map_or_else(
                                |_| {
                                    println!(
                                        "no find address-> outpoint: {}, output.script_pubkey: {}",
                                        outpoint, output.script_pubkey
                                    );
                                    "".to_string()
                                },
                                |address| address.to_string(),
                            );
                        // .ok()
                        // .map(|address| address.to_string())
                        // .ok_or_else(|| {
                        //   anyhow::Error::msg(format!("output.script_pubkey: {}", output.script_pubkey))
                        // })?;
                        Ok(api::OrdxBlockInscription {
                            genesesaddress: geneses_address,
                            inscription: ordx_inscription,
                            // output: ordx_output.unwrap_or_default(),
                        })
                    })
                    .collect::<Result<Vec<api::OrdxBlockInscription>, ServerError>>()?,
            })
        }
        .into_response())
    }

    async fn ordx_block_tx_outputs_inscriptions(
        Extension(server_config): Extension<Arc<ServerConfig>>,
        Extension(index): Extension<Arc<Index>>,
        Path(DeserializeFromStr(block_height)): Path<DeserializeFromStr<u32>>,
    ) -> ServerResult<Response> {
        let block = index
            .get_block_by_height(block_height)?
            .ok_or_not_found(|| format!("block {block_height}"))?;

        let mut inscription_id_list: Vec<InscriptionId> = Vec::new();
        for tx in block.txdata.iter() {
            let txid = tx.txid();
            let output_len = tx.output.len();
            for vout_index in 0..output_len {
                let outpoint = OutPoint::new(txid, vout_index as u32);
                let inscriptions = index
                    .get_inscriptions_on_output(outpoint)
                    .unwrap_or_default();

                for inscription_id in &inscriptions {
                    // skip same tx for geneses new inscription, only update exist inscription with transfer
                    if inscription_id.txid != txid {
                        inscription_id_list.push(*inscription_id);
                    }
                }
            }
        }

        Ok({
            Json(api::OrdxBlockTxOutputInscriptions {
                height: block_height,
                inscriptions: inscription_id_list
                    .par_iter()
                    .map(|inscription_id| {
                        let query_inscription_id = query::Inscription::Id(*inscription_id);
                        let info = Index::inscription_info(&index, query_inscription_id)?
                            .ok_or_not_found(|| format!("inscription {query_inscription_id}"))?;

                        // api inscription
                        Ok(api::OrdxInscription {
                            address: info
                                .output
                                .as_ref()
                                .and_then(|o| {
                                    server_config
                                        .chain
                                        .address_from_script(&o.script_pubkey)
                                        .ok()
                                })
                                .map(|address| address.to_string()),

                            children: info.children,
                            content_length: info.inscription.content_length(),
                            content_type: info.inscription.content_type().map(|s| s.to_string()),
                            fee: info.entry.fee,
                            height: info.entry.height,
                            id: info.entry.id,
                            next: info.next,
                            number: info.entry.inscription_number,
                            parent: info.parent,
                            previous: info.previous,
                            sat: info.entry.sat,
                            satpoint: info.satpoint,
                            timestamp: timestamp(info.entry.timestamp).timestamp(),
                            value: info.output.as_ref().map(|o| o.value),
                        })
                    })
                    .collect::<Result<Vec<api::OrdxInscription>, ServerError>>()?,
            })
        }
        .into_response())
    }
}
