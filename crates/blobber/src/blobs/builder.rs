use crate::{
    AsyncBlobSource, BlobCacher, BlobFetcher, BlobFetcherConfig, BlobSource,
    sources::{BeaconBlobSource, BlobExplorerSource, PylonBlobSource},
};

/// Errors that can occur while building the [`BlobFetcher`] with a
/// [`BlobFetcherBuilder`].
#[derive(Debug, thiserror::Error)]
pub enum BuilderError {
    /// The URL provided was invalid.
    #[error("invalid URL provided")]
    Url(#[from] url::ParseError),
    /// The client failed to build.
    #[error("failed to build client: {0}")]
    Client(#[from] reqwest::Error),
}

/// Builder for the [`BlobFetcher`].
///
/// Add synchronous and asynchronous blob sources, then call [`build`] to
/// produce a [`BlobFetcher`] or [`build_cache`] for a [`BlobCacher`].
///
/// [`build`]: BlobFetcherBuilder::build
/// [`build_cache`]: BlobFetcherBuilder::build_cache
#[derive(Default)]
pub struct BlobFetcherBuilder {
    sync_sources: Vec<Box<dyn BlobSource>>,
    async_sources: Vec<Box<dyn AsyncBlobSource>>,
}

impl core::fmt::Debug for BlobFetcherBuilder {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("BlobFetcherBuilder")
            .field("sync_sources", &self.sync_sources.len())
            .field("async_sources", &self.async_sources.len())
            .finish()
    }
}

impl BlobFetcherBuilder {
    /// Adds a synchronous blob source.
    pub fn with_source(mut self, source: impl BlobSource + 'static) -> Self {
        self.sync_sources.push(Box::new(source));
        self
    }

    /// Adds an asynchronous blob source.
    pub fn with_async_source(mut self, source: impl AsyncBlobSource + 'static) -> Self {
        self.async_sources.push(Box::new(source));
        self
    }

    /// Configures standard remote sources from a [`BlobFetcherConfig`].
    ///
    /// This constructs a [`BlobExplorerSource`], and optionally a
    /// [`BeaconBlobSource`] and [`PylonBlobSource`] depending on whether
    /// the config provides CL and Pylon URLs.
    pub fn with_config(
        self,
        config: &BlobFetcherConfig,
        client: reqwest::Client,
    ) -> Result<Self, BuilderError> {
        let explorer = foundry_blob_explorers::Client::new_with_client(
            config.blob_explorer_url(),
            client.clone(),
        );
        let this = self.with_async_source(BlobExplorerSource::new(explorer));

        let this = match config.cl_url() {
            Some(cl) => {
                let url = url::Url::parse(cl)?;
                this.with_async_source(BeaconBlobSource::new(client.clone(), url))
            }
            None => this,
        };

        match config.pylon_url() {
            Some(pylon) => {
                let url = url::Url::parse(pylon)?;
                Ok(this.with_async_source(PylonBlobSource::new(client, url)))
            }
            None => Ok(this),
        }
    }

    /// Build the [`BlobFetcher`].
    pub fn build(self) -> BlobFetcher {
        BlobFetcher::new(self.sync_sources, self.async_sources)
    }

    /// Build a [`BlobCacher`] wrapping the constructed [`BlobFetcher`].
    pub fn build_cache(self) -> BlobCacher {
        BlobCacher::new(self.build())
    }
}
