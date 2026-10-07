//! Streaming product feed generator for Heureka, Zbozi.cz and Google Merchant.
//!
//! The crate reads a JSON catalog snapshot and writes an XML product feed.
//! Products stream through memory one at a time: the JSON reader yields one
//! product, the XML writer emits the matching feed item, and the product is
//! dropped before the next one is parsed.
//!
//! # Example
//!
//! ```
//! use feedforge::{generate, Config};
//!
//! let config = Config::from_toml_str(
//!     r#"
//! portal = "heureka"
//! output = "feed.xml"
//! currency = "CZK"
//! "#,
//! )
//! .unwrap();
//! let catalog = r#"{
//!   "shop": {
//!     "name": "Example Shop",
//!     "url": "https://example.test",
//!     "email": "shop@example.test"
//!   },
//!   "products": []
//! }"#;
//!
//! let mut output = Vec::new();
//! let outcome = generate(&config, catalog.as_bytes(), &mut output).unwrap();
//!
//! assert_eq!(outcome.written, 0);
//! assert_eq!(outcome.skipped_count(), 0);
//! ```
//!
//! Output is always UTF-8 with stable product order. Identical input and
//! configuration produce identical output bytes.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::fmt;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use quick_xml::Writer;
use quick_xml::events::{BytesDecl, BytesEnd, BytesStart, BytesText, Event};
use serde::de::{self, DeserializeSeed, IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

const ZBOZI_NAMESPACE: &str = "http://www.zbozi.cz/ns/offer/1.0";
const GOOGLE_NAMESPACE: &str = "http://base.google.com/ns/1.0";
const CATEGORY_SEPARATOR: &str = " > ";
const DEFAULT_REQUIRED_FIELDS: &[&str] = &[
    "id",
    "title",
    "description",
    "price",
    "currency",
    "availability",
    "url",
    "image",
];
const AVAILABILITY_NAMES: &[&str] = &["in_stock", "out_of_stock", "preorder", "discontinued"];
const FIELD_NAMES: &[&str] = &[
    "id",
    "title",
    "description",
    "price",
    "currency",
    "availability",
    "url",
    "image",
    "ean",
    "brand",
    "category",
];
const HEUREKA_AVAILABILITY: [(&str, &str); 4] = [
    ("in_stock", "0"),
    ("out_of_stock", "1"),
    ("preorder", "2"),
    ("discontinued", "3"),
];
const ZBOZI_AVAILABILITY: [(&str, &str); 4] = [
    ("in_stock", "0"),
    ("out_of_stock", "-1"),
    ("preorder", "8"),
    ("discontinued", "-1"),
];
const GOOGLE_AVAILABILITY: [(&str, &str); 4] = [
    ("in_stock", "in_stock"),
    ("out_of_stock", "out_of_stock"),
    ("preorder", "preorder"),
    ("discontinued", "out_of_stock"),
];

/// Target shopping portal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Portal {
    /// Czech price comparison portal Heureka.cz.
    Heureka,
    /// Czech shopping portal Zbozi.cz.
    Zbozi,
    /// Google Merchant Center product feed.
    Google,
}

impl Portal {
    /// Returns the portal name used in configuration files.
    pub fn as_str(self) -> &'static str {
        match self {
            Portal::Heureka => "heureka",
            Portal::Zbozi => "zbozi",
            Portal::Google => "google",
        }
    }

    /// Returns the separator used between category path segments.
    pub fn category_separator(self) -> &'static str {
        match self {
            Portal::Heureka | Portal::Zbozi => " | ",
            Portal::Google => CATEGORY_SEPARATOR,
        }
    }

    /// Returns the default required field names for the portal.
    pub fn default_required_fields(self) -> &'static [&'static str] {
        DEFAULT_REQUIRED_FIELDS
    }
}

/// Availability of a product in the source catalog.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Availability {
    /// The product is in stock.
    InStock,
    /// The product is out of stock.
    OutOfStock,
    /// The product can be ordered before release.
    Preorder,
    /// The product is discontinued.
    Discontinued,
}

impl Availability {
    /// Returns the internal name used in configuration files.
    pub fn as_str(self) -> &'static str {
        match self {
            Availability::InStock => "in_stock",
            Availability::OutOfStock => "out_of_stock",
            Availability::Preorder => "preorder",
            Availability::Discontinued => "discontinued",
        }
    }
}

/// Shop metadata from the catalog header.
#[derive(Debug, Clone, Deserialize)]
pub struct Shop {
    /// Shop name.
    pub name: String,
    /// Shop homepage URL.
    pub url: String,
    /// Contact email address.
    pub email: String,
}

/// One product from the catalog snapshot.
#[derive(Debug, Clone, Deserialize)]
pub struct Product {
    /// Product identifier, unique within the shop.
    #[serde(default)]
    pub id: String,
    /// Product title.
    #[serde(default)]
    pub title: String,
    /// Full product description.
    #[serde(default)]
    pub description: String,
    /// Price in minor currency units, for example haléře or cents.
    #[serde(default)]
    pub price_minor: Option<i64>,
    /// Currency of the price.
    #[serde(default)]
    pub currency: String,
    /// Product availability.
    #[serde(default)]
    pub availability: Option<Availability>,
    /// Product page URL.
    #[serde(default)]
    pub url: String,
    /// Product image URLs in display order.
    #[serde(default)]
    pub image_urls: Vec<String>,
    /// Category path from the root category to the product category.
    #[serde(default)]
    pub category_path: Vec<String>,
    /// European Article Number.
    #[serde(default)]
    pub ean: Option<String>,
    /// Manufacturer or brand name.
    #[serde(default)]
    pub brand: Option<String>,
    /// Shipping weight in grams.
    #[serde(default)]
    pub weight_grams: Option<u64>,
    /// Free-form product parameters as name-value pairs.
    #[serde(default)]
    pub params: BTreeMap<String, Value>,
}

/// A complete catalog snapshot.
#[derive(Debug, Clone, Deserialize)]
pub struct Catalog {
    /// Shop metadata.
    pub shop: Shop,
    /// Products in catalog order.
    pub products: Vec<Product>,
}

/// One skipped product and the reasons why it was skipped.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct SkippedEntry {
    /// Zero-based position of the product in the input catalog.
    pub index: usize,
    /// Product identifier when the input provides one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// Machine-readable skip reasons.
    pub reasons: Vec<String>,
}

/// Result of a feed generation run.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct Outcome {
    /// Number of products written to the feed.
    pub written: usize,
    /// Skipped products in input order.
    pub skipped: Vec<SkippedEntry>,
}

impl Outcome {
    /// Returns the number of skipped products.
    pub fn skipped_count(&self) -> usize {
        self.skipped.len()
    }
}

/// Feed generation configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Target portal.
    pub portal: Portal,
    /// Output feed path. The command line `--output` option overrides it.
    #[serde(default)]
    pub output: Option<PathBuf>,
    /// Feed currency as a three-letter code. Heureka and Zbozi require CZK.
    pub currency: String,
    /// Maximum description length in Unicode scalar values.
    #[serde(default = "default_max_description_chars")]
    pub max_description_chars: usize,
    /// Availability mapping overrides from internal name to portal value.
    #[serde(default)]
    pub availability: BTreeMap<String, String>,
    /// Category path prefix rewrites from source path to portal path.
    #[serde(default)]
    pub category_map: BTreeMap<String, String>,
    /// Required field names. When set, the list replaces the portal default.
    #[serde(default)]
    pub required_fields: Option<Vec<String>>,
}

impl Config {
    /// Parses and validates a configuration from TOML text.
    ///
    /// # Errors
    ///
    /// Returns an error when the TOML is malformed or a value is invalid for
    /// the selected portal.
    pub fn from_toml_str(text: &str) -> Result<Config> {
        let config: Config = toml::from_str(text).context("invalid configuration TOML")?;
        config.validate()?;
        Ok(config)
    }

    /// Reads, parses and validates a configuration file.
    ///
    /// # Errors
    ///
    /// Returns an error when the file cannot be read or the content is
    /// invalid.
    pub fn from_file(path: &Path) -> Result<Config> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("cannot read configuration file {}", path.display()))?;
        Config::from_toml_str(&text)
    }

    /// Validates the configuration values.
    ///
    /// # Errors
    ///
    /// Returns an error when the currency, description limit, availability
    /// mapping, category mapping or required field list is invalid.
    pub fn validate(&self) -> Result<()> {
        Settings::new(self).map(|_| ())
    }
}

fn default_max_description_chars() -> usize {
    2000
}

/// Formats a price in minor units with exactly two fraction digits.
///
/// The conversion uses integer arithmetic only.
///
/// ```
/// assert_eq!(feedforge::format_price_minor(12345), "123.45");
/// assert_eq!(feedforge::format_price_minor(5), "0.05");
/// ```
pub fn format_price_minor(price_minor: i64) -> String {
    let sign = if price_minor < 0 { "-" } else { "" };
    let absolute = price_minor.unsigned_abs();
    format!("{sign}{}.{:02}", absolute / 100, absolute % 100)
}

/// Generates a feed from a JSON catalog and writes the XML to `output`.
///
/// The catalog object must contain `shop` before `products`. Products are
/// parsed and written one at a time, so only a single product is held in
/// memory during generation. Products that fail validation are skipped and
/// listed in the returned [`Outcome`].
///
/// # Errors
///
/// Returns an error when the configuration is invalid, the JSON catalog is
/// malformed, an I/O operation fails or the `shop` value is missing or
/// misplaced.
pub fn generate<R: Read, W: Write>(config: &Config, input: R, output: W) -> Result<Outcome> {
    let settings = Settings::new(config)?;
    let mut feed = FeedWriter::new(output, settings.portal);
    let mut state = StreamState::default();
    let mut deserializer = serde_json::Deserializer::from_reader(input);
    let result = CatalogSeed {
        feed: &mut feed,
        settings: &settings,
        state: &mut state,
    }
    .deserialize(&mut deserializer);
    if let Err(error) = result {
        if let Some(fatal) = state.fatal.take() {
            return Err(fatal);
        }
        return Err(anyhow::Error::new(error).context("invalid catalog JSON"));
    }
    if let Some(fatal) = state.fatal.take() {
        return Err(fatal);
    }
    feed.finish()?;
    Ok(state.outcome)
}

struct Settings {
    portal: Portal,
    currency: String,
    max_description_chars: usize,
    availability: BTreeMap<&'static str, String>,
    category_map: Vec<(Vec<String>, Vec<String>)>,
    required: Vec<String>,
}

impl Settings {
    fn new(config: &Config) -> Result<Settings> {
        let currency = config.currency.trim().to_ascii_uppercase();
        if currency.len() != 3
            || !currency
                .chars()
                .all(|character| character.is_ascii_alphabetic())
        {
            bail!(
                "currency must be a three-letter code, got {:?}",
                config.currency
            );
        }
        if matches!(config.portal, Portal::Heureka | Portal::Zbozi) && currency != "CZK" {
            bail!(
                "portal {} requires CZK currency, got {currency}",
                config.portal.as_str()
            );
        }
        if config.max_description_chars == 0 {
            bail!("max_description_chars must be at least 1");
        }
        let mut availability: BTreeMap<&'static str, String> = default_availability(config.portal)
            .iter()
            .map(|(key, value)| (*key, (*value).to_string()))
            .collect();
        for (key, value) in &config.availability {
            let key = key.trim();
            let Some(name) = AVAILABILITY_NAMES.iter().copied().find(|name| *name == key) else {
                bail!(
                    "unknown availability key {key:?}; expected one of {}",
                    AVAILABILITY_NAMES.join(", ")
                );
            };
            let value = value.trim();
            if value.is_empty() {
                bail!("availability value for {key:?} must not be empty");
            }
            availability.insert(name, value.to_string());
        }
        let mut category_map = Vec::with_capacity(config.category_map.len());
        for (from, to) in &config.category_map {
            category_map.push((parse_category_path(from)?, parse_category_path(to)?));
        }
        category_map.sort_by(|left, right| {
            right
                .0
                .len()
                .cmp(&left.0.len())
                .then_with(|| left.0.cmp(&right.0))
        });
        let required = match &config.required_fields {
            Some(fields) => {
                let mut required: Vec<String> = Vec::with_capacity(fields.len());
                for field in fields {
                    let field = field.trim();
                    if !FIELD_NAMES.contains(&field) {
                        bail!(
                            "unknown required field {field:?}; expected one of {}",
                            FIELD_NAMES.join(", ")
                        );
                    }
                    if !required.iter().any(|known| known == field) {
                        required.push(field.to_string());
                    }
                }
                required
            }
            None => config
                .portal
                .default_required_fields()
                .iter()
                .map(|field| (*field).to_string())
                .collect(),
        };
        Ok(Settings {
            portal: config.portal,
            currency,
            max_description_chars: config.max_description_chars,
            availability,
            category_map,
            required,
        })
    }

    fn map_availability(&self, availability: Availability) -> String {
        self.availability
            .get(availability.as_str())
            .cloned()
            .unwrap_or_else(|| availability.as_str().to_string())
    }
}

fn default_availability(portal: Portal) -> &'static [(&'static str, &'static str)] {
    match portal {
        Portal::Heureka => &HEUREKA_AVAILABILITY,
        Portal::Zbozi => &ZBOZI_AVAILABILITY,
        Portal::Google => &GOOGLE_AVAILABILITY,
    }
}

fn parse_category_path(path: &str) -> Result<Vec<String>> {
    let parts: Vec<String> = path
        .split(CATEGORY_SEPARATOR)
        .map(|part| part.trim().to_string())
        .collect();
    if parts.iter().any(|part| part.is_empty()) {
        bail!("category map path {path:?} contains an empty segment");
    }
    Ok(parts)
}

struct FeedWriter<W: Write> {
    writer: Writer<W>,
    portal: Portal,
}

enum WriteOutcome {
    Written,
    Skipped(Vec<String>),
}

impl<W: Write> FeedWriter<W> {
    fn new(output: W, portal: Portal) -> FeedWriter<W> {
        FeedWriter {
            writer: Writer::new(output),
            portal,
        }
    }

    fn begin(&mut self, shop: &Shop) -> Result<()> {
        self.writer
            .write_event(Event::Decl(BytesDecl::new("1.0", Some("UTF-8"), None)))?;
        match self.portal {
            Portal::Heureka | Portal::Zbozi => {
                let mut root = BytesStart::new("SHOP");
                if self.portal == Portal::Zbozi {
                    root.push_attribute(("xmlns", ZBOZI_NAMESPACE));
                }
                self.writer.write_event(Event::Start(root))?;
            }
            Portal::Google => {
                if !is_valid_url(&shop.url) {
                    bail!("shop.url is not a valid http(s) URL");
                }
                let rss = BytesStart::new("rss")
                    .with_attributes([("version", "2.0"), ("xmlns:g", GOOGLE_NAMESPACE)]);
                self.writer.write_event(Event::Start(rss))?;
                self.start("channel")?;
                self.element("title", shop.name.trim())?;
                self.element("link", shop.url.trim())?;
                self.element("description", shop.name.trim())?;
            }
        }
        Ok(())
    }

    fn finish(mut self) -> Result<()> {
        match self.portal {
            Portal::Heureka | Portal::Zbozi => self.end("SHOP")?,
            Portal::Google => {
                self.end("channel")?;
                self.end("rss")?;
            }
        }
        self.writer.get_mut().flush()?;
        Ok(())
    }

    fn write_product(&mut self, product: &Product, settings: &Settings) -> Result<WriteOutcome> {
        let reasons = validate_product(product, settings);
        if !reasons.is_empty() {
            return Ok(WriteOutcome::Skipped(reasons));
        }
        match settings.portal {
            Portal::Heureka | Portal::Zbozi => self.write_catalog_item(product, settings)?,
            Portal::Google => self.write_google_item(product, settings)?,
        }
        Ok(WriteOutcome::Written)
    }

    fn write_catalog_item(&mut self, product: &Product, settings: &Settings) -> Result<()> {
        self.start("SHOPITEM")?;
        let title = product.title.trim();
        if !title.is_empty() {
            self.element("PRODUCT", title)?;
            self.element("PRODUCTNAME", title)?;
        }
        let description =
            truncate_chars(product.description.trim(), settings.max_description_chars);
        if !description.is_empty() {
            self.element("DESCRIPTION", &description)?;
        }
        let url = product.url.trim();
        if !url.is_empty() {
            self.element("URL", url)?;
        }
        if let Some(price) = product.price_minor {
            self.element("PRICE_VAT", &format_price_minor(price))?;
        }
        self.element("CURRENCY", &settings.currency)?;
        if let Some(availability) = product.availability {
            let tag = if self.portal == Portal::Zbozi {
                "DELIVERY_DATE"
            } else {
                "AVAILABILITY"
            };
            self.element(tag, &settings.map_availability(availability))?;
        }
        let category = map_category(&product.category_path, settings);
        if !category.is_empty() {
            self.element("CATEGORYTEXT", &category)?;
        }
        if let Some(ean) = non_empty(product.ean.as_deref()) {
            self.element("EAN", ean)?;
        }
        if let Some(brand) = non_empty(product.brand.as_deref()) {
            self.element("MANUFACTURER", brand)?;
        }
        for image in &product.image_urls {
            self.element("IMGURL", image.trim())?;
        }
        let value_tag = if self.portal == Portal::Zbozi {
            "VAL"
        } else {
            "PARAM_VAL"
        };
        for (name, value) in &product.params {
            let Some(text) = param_value(value) else {
                continue;
            };
            self.start("PARAM")?;
            self.element("PARAM_NAME", name)?;
            self.element(value_tag, &text)?;
            self.end("PARAM")?;
        }
        self.end("SHOPITEM")?;
        Ok(())
    }

    fn write_google_item(&mut self, product: &Product, settings: &Settings) -> Result<()> {
        self.start("item")?;
        let id = product.id.trim();
        if !id.is_empty() {
            self.element("g:id", id)?;
        }
        let title = product.title.trim();
        if !title.is_empty() {
            self.element("g:title", title)?;
        }
        let description =
            truncate_chars(product.description.trim(), settings.max_description_chars);
        if !description.is_empty() {
            self.element("g:description", &description)?;
        }
        let url = product.url.trim();
        if !url.is_empty() {
            self.element("g:link", url)?;
        }
        if let Some(first) = product.image_urls.first() {
            self.element("g:image_link", first.trim())?;
            for image in product.image_urls.iter().skip(1) {
                self.element("g:additional_image_link", image.trim())?;
            }
        }
        if let Some(price) = product.price_minor {
            let formatted = format!("{} {}", format_price_minor(price), settings.currency);
            self.element("g:price", &formatted)?;
        }
        if let Some(availability) = product.availability {
            self.element("g:availability", &settings.map_availability(availability))?;
        }
        if let Some(brand) = non_empty(product.brand.as_deref()) {
            self.element("g:brand", brand)?;
        }
        if let Some(ean) = non_empty(product.ean.as_deref()) {
            self.element("g:gtin", ean)?;
        }
        self.element("g:condition", "new")?;
        let category = map_category(&product.category_path, settings);
        if !category.is_empty() {
            self.element("g:product_type", &category)?;
        }
        self.end("item")
    }

    fn start(&mut self, name: &str) -> Result<()> {
        self.writer
            .write_event(Event::Start(BytesStart::new(name)))?;
        Ok(())
    }

    fn end(&mut self, name: &str) -> Result<()> {
        self.writer.write_event(Event::End(BytesEnd::new(name)))?;
        Ok(())
    }

    fn element(&mut self, name: &str, text: &str) -> Result<()> {
        self.start(name)?;
        if !text.is_empty() {
            self.writer.write_event(Event::Text(BytesText::new(text)))?;
        }
        self.end(name)
    }
}

fn validate_product(product: &Product, settings: &Settings) -> Vec<String> {
    let mut reasons = Vec::new();
    for field in &settings.required {
        let missing = match field.as_str() {
            "id" => product.id.trim().is_empty(),
            "title" => product.title.trim().is_empty(),
            "description" => product.description.trim().is_empty(),
            "price" => product.price_minor.is_none(),
            "currency" => product.currency.trim().is_empty(),
            "availability" => product.availability.is_none(),
            "url" => product.url.trim().is_empty(),
            "image" => product.image_urls.is_empty(),
            "ean" => product
                .ean
                .as_deref()
                .is_none_or(|value| value.trim().is_empty()),
            "brand" => product
                .brand
                .as_deref()
                .is_none_or(|value| value.trim().is_empty()),
            "category" => product
                .category_path
                .iter()
                .all(|part| part.trim().is_empty()),
            _ => false,
        };
        if missing {
            reasons.push(format!("missing_field:{field}"));
        }
    }
    if product.image_urls.is_empty() {
        push_unique(&mut reasons, "missing_field:image");
    }
    if product.price_minor.is_some_and(|price| price < 0) {
        reasons.push("invalid_price".to_string());
    }
    let url = product.url.trim();
    if !url.is_empty() && !is_valid_url(url) {
        reasons.push("invalid_url:url".to_string());
    }
    if product.image_urls.iter().any(|image| !is_valid_url(image)) {
        reasons.push("invalid_url:image_urls".to_string());
    }
    let currency = product.currency.trim();
    if !currency.is_empty() && !currency.eq_ignore_ascii_case(&settings.currency) {
        reasons.push("currency_mismatch".to_string());
    }
    reasons
}

fn push_unique(reasons: &mut Vec<String>, reason: &str) {
    if !reasons.iter().any(|existing| existing == reason) {
        reasons.push(reason.to_string());
    }
}

fn is_valid_url(value: &str) -> bool {
    let value = value.trim();
    let Some(offset) = url_scheme_end(value) else {
        return false;
    };
    let rest = &value[offset..];
    if rest.is_empty() || rest.chars().any(char::is_whitespace) {
        return false;
    }
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    !host.is_empty()
}

fn url_scheme_end(value: &str) -> Option<usize> {
    if value
        .get(..8)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("https://"))
    {
        return Some(8);
    }
    if value
        .get(..7)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("http://"))
    {
        return Some(7);
    }
    None
}

fn non_empty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|text| !text.is_empty())
}

fn truncate_chars(text: &str, max_chars: usize) -> Cow<'_, str> {
    let end = text
        .char_indices()
        .nth(max_chars)
        .map_or(text.len(), |(index, _)| index);
    Cow::Borrowed(&text[..end])
}

fn param_value(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::String(text) => {
            if text.is_empty() {
                None
            } else {
                Some(text.clone())
            }
        }
        other => Some(other.to_string()),
    }
}

fn map_category(path: &[String], settings: &Settings) -> String {
    let mut components: Vec<String> = path
        .iter()
        .map(|part| part.trim().to_string())
        .filter(|part| !part.is_empty())
        .collect();
    for (from, to) in &settings.category_map {
        if components.len() >= from.len() && components[..from.len()] == from[..] {
            let mut mapped = to.clone();
            mapped.extend(components.drain(from.len()..));
            return mapped.join(settings.portal.category_separator());
        }
    }
    components.join(settings.portal.category_separator())
}

#[derive(Default)]
struct StreamState {
    outcome: Outcome,
    fatal: Option<anyhow::Error>,
}

struct CatalogSeed<'a, W: Write> {
    feed: &'a mut FeedWriter<W>,
    settings: &'a Settings,
    state: &'a mut StreamState,
}

impl<'de, 'a, W: Write> DeserializeSeed<'de> for CatalogSeed<'a, W> {
    type Value = ();

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_map(self)
    }
}

impl<'de, 'a, W: Write> Visitor<'de> for CatalogSeed<'a, W> {
    type Value = ();

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON catalog object")
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut shop_seen = false;
        let mut products_seen = false;
        while let Some(key) = map.next_key::<String>()? {
            match key.as_str() {
                "shop" => {
                    if shop_seen {
                        return Err(de::Error::custom("duplicate `shop` key"));
                    }
                    if products_seen {
                        return Err(de::Error::custom("`shop` must appear before `products`"));
                    }
                    let shop = map.next_value::<Shop>()?;
                    if let Err(error) = self.feed.begin(&shop) {
                        self.state.fatal = Some(error);
                        return Err(de::Error::custom("cannot write feed header"));
                    }
                    shop_seen = true;
                }
                "products" => {
                    if !shop_seen {
                        return Err(de::Error::custom("`shop` must appear before `products`"));
                    }
                    if products_seen {
                        return Err(de::Error::custom("duplicate `products` key"));
                    }
                    products_seen = true;
                    map.next_value_seed(ProductsSeed {
                        feed: &mut *self.feed,
                        settings: self.settings,
                        state: &mut *self.state,
                        index: 0,
                    })?;
                }
                _ => {
                    map.next_value::<IgnoredAny>()?;
                }
            }
        }
        if !shop_seen {
            return Err(de::Error::missing_field("shop"));
        }
        Ok(())
    }
}

struct ProductsSeed<'a, W: Write> {
    feed: &'a mut FeedWriter<W>,
    settings: &'a Settings,
    state: &'a mut StreamState,
    index: usize,
}

impl<'de, 'a, W: Write> DeserializeSeed<'de> for ProductsSeed<'a, W> {
    type Value = ();

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_seq(self)
    }
}

impl<'de, 'a, W: Write> Visitor<'de> for ProductsSeed<'a, W> {
    type Value = ();

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON list of products")
    }

    fn visit_seq<A>(mut self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        while let Some(product) = sequence.next_element::<Product>()? {
            match self.feed.write_product(&product, self.settings) {
                Ok(WriteOutcome::Written) => self.state.outcome.written += 1,
                Ok(WriteOutcome::Skipped(reasons)) => {
                    let id = if product.id.trim().is_empty() {
                        None
                    } else {
                        Some(product.id.clone())
                    };
                    self.state.outcome.skipped.push(SkippedEntry {
                        index: self.index,
                        id,
                        reasons,
                    });
                }
                Err(error) => {
                    self.state.fatal = Some(error);
                    return Err(de::Error::custom("cannot write feed item"));
                }
            }
            self.index += 1;
        }
        Ok(())
    }
}
