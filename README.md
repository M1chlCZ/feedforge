# feedforge

feedforge makes XML product feeds for Heureka, Zbozi.cz and Google Merchant
from a JSON catalog snapshot. The program runs offline. The output is
deterministic. Products stream through memory one at a time, so large catalogs
need little memory.

The package contains a library and the `feedforge` command.

## Install

```sh
cargo install feedforge
```

## Input catalog

The input is one JSON object. The object has a `shop` header and a `products`
list. The `shop` key must come before the `products` key, because the reader
streams products while it parses.

```json
{
  "shop": {
    "name": "Example Shop",
    "url": "https://example.test",
    "email": "shop@example.test"
  },
  "products": [
    {
      "id": "SKU-1",
      "title": "Kávovar",
      "description": "Malý kávovar pro dvě osoby.",
      "price_minor": 12345,
      "currency": "CZK",
      "availability": "in_stock",
      "url": "https://example.test/p/sku-1",
      "image_urls": ["https://example.test/img/sku-1.jpg"],
      "category_path": ["Domov", "Kuchyně"],
      "ean": "8591234567890",
      "brand": "Acme",
      "weight_grams": 1500,
      "params": {"Barva": "červená"}
    }
  ]
}
```

| Field | Type | Description |
| --- | --- | --- |
| `id` | string | Product identifier, unique in the shop. |
| `title` | string | Product title. |
| `description` | string | Full product description. |
| `price_minor` | integer | Price in minor units, for example haléře or cents. |
| `currency` | string | Price currency. |
| `availability` | string | `in_stock`, `out_of_stock`, `preorder` or `discontinued`. |
| `url` | string | Product page URL. It must start with `http://` or `https://`. |
| `image_urls` | list of strings | Product image URLs in display order. |
| `category_path` | list of strings | Category path from the root category. |
| `ean` | string | European Article Number. Optional. |
| `brand` | string | Manufacturer or brand name. Optional. |
| `weight_grams` | integer | Shipping weight in grams. Optional. The writers do not emit it. |
| `params` | object | Free-form name-value pairs. Optional. |

## Configuration

The `--config` option points to a TOML file.

```toml
portal = "heureka"
output = "feed.xml"
currency = "CZK"
max_description_chars = 2000
required_fields = ["id", "title", "description", "price", "currency", "availability", "url", "image"]

[availability]
in_stock = "0"

[category_map]
"Domov > Kuchyně" = "Kuchyně a jídlo"
```

| Key | Type | Default | Description |
| --- | --- | --- | --- |
| `portal` | string | required | `heureka`, `zbozi` or `google`. |
| `output` | path | none | Output feed path. The `--output` option replaces it. |
| `currency` | string | required | Three-letter code. Heureka and Zbozi accept only `CZK`. |
| `max_description_chars` | integer | 2000 | Maximum description length in Unicode characters. |
| `availability` | table | portal defaults | Maps each internal value to a portal value. |
| `category_map` | table | empty | Rewrites category path prefixes. |
| `required_fields` | list | portal defaults | Replaces the default required field list. |

### Availability mapping

The table below shows the default value for each internal status.

| Internal | Heureka | Zbozi | Google |
| --- | --- | --- | --- |
| `in_stock` | `0` | `0` | `in_stock` |
| `out_of_stock` | `1` | `-1` | `out_of_stock` |
| `preorder` | `2` | `8` | `preorder` |
| `discontinued` | `3` | `-1` | `out_of_stock` |

You can replace each default value in the `[availability]` table.

### Category mapping

A `category_map` entry rewrites a category path prefix. Write both sides as
path segments with ` > ` between them. The longest matching prefix wins. The
writer then joins the result with the portal separator.

```toml
[category_map]
"Domov" = "Domácí potřeby"
"Domov > Kuchyně" = "Kuchyně a jídlo"
```

A product with the path `Domov > Kuchyně > Hrnce` becomes
`Kuchyně a jídlo | Hrnce` for Heureka and `Kuchyně a jídlo > Hrnce` for Google.

### Required fields

The default required list is `id`, `title`, `description`, `price`,
`currency`, `availability`, `url` and `image`. The setting `required_fields`
replaces this list. A product without images is always skipped, also when the
list does not contain `image`. Every listed value must be one of:
`id`, `title`, `description`, `price`, `currency`, `availability`, `url`,
`image`, `ean`, `brand` and `category`.

## Portal output

| Data | Heureka and Zbozi | Google |
| --- | --- | --- |
| Identifier | not emitted | `g:id` |
| Title | `PRODUCT`, `PRODUCTNAME` | `g:title` |
| Description | `DESCRIPTION` | `g:description` |
| URL | `URL` | `g:link` |
| Images | one `IMGURL` per image | `g:image_link`, then `g:additional_image_link` |
| Price | `PRICE_VAT` | `g:price` |
| Currency | `CURRENCY` | inside `g:price` |
| Availability | `AVAILABILITY` or `DELIVERY_DATE` | `g:availability` |
| Category | `CATEGORYTEXT` | `g:product_type` |
| EAN | `EAN` | `g:gtin` |
| Brand | `MANUFACTURER` | `g:brand` |
| Parameters | `PARAM` entries | not emitted |
| Condition | not emitted | `g:condition` = `new` |

The Zbozi writer has three differences:

1. The `SHOP` root carries `xmlns="http://www.zbozi.cz/ns/offer/1.0"`.
2. Zbozi writes availability in `DELIVERY_DATE`. Heureka writes it in
   `AVAILABILITY`.
3. Zbozi writes `VAL` inside `PARAM`. Heureka writes `PARAM_VAL`.

The Google writer makes one RSS 2.0 document with the `g` namespace.

Prices use integer arithmetic only. `12345` minor units become `123.45`.
Descriptions truncate at the character limit on a character boundary.

## Command line

```text
feedforge --config feed.toml --input catalog.json [--output feed.xml] [--report report.json] [--strict]
```

| Option | Description |
| --- | --- |
| `--config FILE` | TOML configuration file. Required. |
| `--input FILE` | JSON catalog snapshot. Required. |
| `--output FILE` | Output path. Replaces the config `output` value. |
| `--report FILE` | Write a JSON report of the run. |
| `--strict` | Exit with code 1 when any product is skipped. |

| Exit code | Meaning |
| --- | --- |
| 0 | Success. |
| 1 | Success, but at least one product was skipped in `--strict` mode. |
| 2 | Configuration error, input error or I/O error. |

## Report

The report lists the number of written products and every skipped product
with the skip reasons.

```json
{
  "written": 12,
  "skipped": [
    {
      "index": 3,
      "id": "SKU-4",
      "reasons": ["missing_field:description", "invalid_url:image_urls"]
    }
  ]
}
```

| Reason | Meaning |
| --- | --- |
| `missing_field:NAME` | A required field is absent or empty. |
| `invalid_url:url` | The product URL is invalid. |
| `invalid_url:image_urls` | At least one image URL is invalid. |
| `invalid_price` | The price is negative. |
| `currency_mismatch` | The product currency differs from the config currency. |

A product with an empty `image_urls` list is always skipped with
`missing_field:image`.

## Determinism

Identical input and configuration produce identical output bytes. Products
keep the input order. Parameters sort by name. The output holds no timestamps.
The report is deterministic too.

## Development

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo build --release
```

## License

MIT. See [LICENSE](LICENSE).
