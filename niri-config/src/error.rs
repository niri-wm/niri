use std::error::Error;
use std::fmt;
use std::path::PathBuf;

use miette::Diagnostic;

#[derive(Debug)]
pub struct ConfigParseResult<T, E> {
    pub config: Result<T, E>,

    // We always try to return includes for the file watcher.
    //
    // If the main config is valid, but an included file fails to parse, config will be an Err(),
    // but includes will still be filled, so that fixing just the included file is enough to
    // trigger a reload.
    pub includes: Vec<PathBuf>,
}

/// Error type that chains main errors with include errors.
///
/// Allows miette's Report formatting to have main + include errors all in one.
#[derive(Debug)]
pub struct ConfigIncludeError {
    pub main: knuffel::Error,
    pub includes: Vec<knuffel::Error>,
}

impl<T, E> ConfigParseResult<T, E> {
    pub fn from_err(err: E) -> Self {
        Self {
            config: Err(err),
            includes: Vec::new(),
        }
    }

    pub fn map_config_res<U, V>(
        self,
        f: impl FnOnce(Result<T, E>) -> Result<U, V>,
    ) -> ConfigParseResult<U, V> {
        ConfigParseResult {
            config: f(self.config),
            includes: self.includes,
        }
    }
}

impl fmt::Display for ConfigIncludeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.main, f)
    }
}

impl Error for ConfigIncludeError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.main.source()
    }
}

impl Diagnostic for ConfigIncludeError {
    fn code<'a>(&'a self) -> Option<Box<dyn fmt::Display + 'a>> {
        self.main.code()
    }

    fn severity(&self) -> Option<miette::Severity> {
        self.main.severity()
    }

    fn help<'a>(&'a self) -> Option<Box<dyn fmt::Display + 'a>> {
        self.main.help()
    }

    fn url<'a>(&'a self) -> Option<Box<dyn fmt::Display + 'a>> {
        self.main.url()
    }

    fn source_code(&self) -> Option<&dyn miette::SourceCode> {
        self.main.source_code()
    }

    fn labels(&self) -> Option<Box<dyn Iterator<Item = miette::LabeledSpan> + '_>> {
        self.main.labels()
    }

    fn diagnostic_source(&self) -> Option<&dyn Diagnostic> {
        self.main.diagnostic_source()
    }

    fn related<'a>(&'a self) -> Option<Box<dyn Iterator<Item = &'a dyn Diagnostic> + 'a>> {
        let main_related = self.main.related();
        let includes_iter = self.includes.iter().map(|err| err as &'a dyn Diagnostic);

        let iter: Box<dyn Iterator<Item = &'a dyn Diagnostic> + 'a> = match main_related {
            Some(main) => Box::new(main.chain(includes_iter)),
            None => Box::new(includes_iter),
        };

        Some(iter)
    }
}
