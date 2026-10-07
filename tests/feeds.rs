mod support;

use feedforge::{Catalog, Config, Outcome, format_price_minor, generate};
use support::{Node, parse};

const CATALOG: &str = include_str!("fixtures/catalog.json");

const ESCAPING_CATALOG: &str = r#"{
  "shop": {
    "name": "A & B <Shop>",
    "url": "https://example.test",
    "email": "shop@example.test"
  },
  "products": [
    {
      "id": "SKU\"1",
      "title": "Kávovar & <čajník>",
      "description": "Příliš \"žluťoučký\" kůň & <pes>",
      "price_minor": 1,
      "currency": "CZK",
      "availability": "preorder",
      "url": "https://example.test/p?a=1&b=2",
      "image_urls": ["https://example.test/i.jpg?a=1&b=2"],
      "category_path": ["Dům & byt"],
      "brand": "Acme & Sons",
      "params": {
        "Vlastnost & klíč": "hodnota <x> & \"y\""
      }
    }
  ]
}"#;

fn config_text(portal: &str, extra: &str) -> String {
    format!("portal = \"{portal}\"\noutput = \"feed.xml\"\ncurrency = \"CZK\"\n{extra}")
}

fn run(config: &str, catalog: &str) -> (String, Outcome) {
    let config = Config::from_toml_str(config).expect("valid config");
    let mut output = Vec::new();
    let outcome = generate(&config, catalog.as_bytes(), &mut output).expect("feed generated");
    let xml = String::from_utf8(output).expect("UTF-8 output");
    (xml, outcome)
}

fn valid_product(id: &str) -> serde_json::Value {
    serde_json::json!({
        "id": id,
        "title": format!("Product {id}"),
        "description": "A valid product.",
        "price_minor": 100,
        "currency": "CZK",
        "availability": "in_stock",
        "url": format!("https://example.test/p/{id}"),
        "image_urls": [format!("https://example.test/i/{id}.jpg")],
        "category_path": []
    })
}

fn catalog_with(products: serde_json::Value) -> String {
    format!(
        r#"{{"shop":{{"name":"Example Shop","url":"https://example.test","email":"shop@example.test"}},"products":{products}}}"#
    )
}

#[test]
fn heureka_feed_has_expected_elements() {
    let (xml, outcome) = run(&config_text("heureka", ""), CATALOG);
    assert_eq!(outcome.written, 2);
    assert_eq!(outcome.skipped_count(), 0);
    let root = parse(&xml);
    assert_eq!(root.name, "SHOP");
    let items: Vec<&Node> = root.children_named("SHOPITEM").collect();
    assert_eq!(items.len(), 2);
    let first = items[0];
    assert_eq!(
        first.text_of("PRODUCT"),
        "Kávovar & čajník <Premium> „edice“"
    );
    assert_eq!(first.text_of("PRODUCTNAME"), first.text_of("PRODUCT"));
    assert!(
        first
            .text_of("DESCRIPTION")
            .starts_with("Příliš žluťoučký kůň")
    );
    assert_eq!(first.text_of("URL"), "https://example.test/p/sku-1");
    assert_eq!(first.text_of("PRICE_VAT"), "123.45");
    assert_eq!(first.text_of("CURRENCY"), "CZK");
    assert_eq!(first.text_of("AVAILABILITY"), "0");
    assert_eq!(first.text_of("CATEGORYTEXT"), "Domov | Kuchyně");
    assert_eq!(first.text_of("EAN"), "8591234567890");
    assert_eq!(first.text_of("MANUFACTURER"), "Acme & Sons");
    let images: Vec<String> = first
        .children_named("IMGURL")
        .map(|node| node.text.clone())
        .collect();
    assert_eq!(
        images,
        vec![
            "https://example.test/img/sku-1-a.jpg",
            "https://example.test/img/sku-1-b.jpg?x=1&y=2"
        ]
    );
    let params: Vec<&Node> = first.children_named("PARAM").collect();
    assert_eq!(params.len(), 2);
    assert_eq!(params[0].text_of("PARAM_NAME"), "Barva");
    assert_eq!(params[0].text_of("PARAM_VAL"), "červená");
    assert_eq!(params[1].text_of("PARAM_NAME"), "Materiál");
    assert_eq!(params[1].text_of("PARAM_VAL"), "nerez");
    assert_eq!(items[1].text_of("AVAILABILITY"), "1");
    assert_eq!(items[1].text_of("PRICE_VAT"), "9.99");
}

#[test]
fn zbozi_feed_documents_portal_differences() {
    let (xml, outcome) = run(&config_text("zbozi", ""), CATALOG);
    assert_eq!(outcome.written, 2);
    let root = parse(&xml);
    assert_eq!(root.name, "SHOP");
    assert_eq!(root.attr("xmlns"), Some("http://www.zbozi.cz/ns/offer/1.0"));
    let first = root.children_named("SHOPITEM").next().unwrap();
    assert!(!first.has("AVAILABILITY"));
    assert_eq!(first.text_of("DELIVERY_DATE"), "0");
    let params: Vec<&Node> = first.children_named("PARAM").collect();
    assert_eq!(params[0].text_of("VAL"), "červená");
    assert!(!params[0].has("PARAM_VAL"));
    assert_eq!(first.text_of("CURRENCY"), "CZK");
}

#[test]
fn google_feed_has_rss_structure() {
    let (xml, outcome) = run(&config_text("google", ""), CATALOG);
    assert_eq!(outcome.written, 2);
    let rss = parse(&xml);
    assert_eq!(rss.name, "rss");
    assert_eq!(rss.attr("version"), Some("2.0"));
    assert_eq!(rss.attr("xmlns:g"), Some("http://base.google.com/ns/1.0"));
    let channel = rss.child("channel");
    assert_eq!(channel.text_of("title"), "Example Shop");
    assert_eq!(channel.text_of("link"), "https://example.test");
    let items: Vec<&Node> = channel.children_named("item").collect();
    assert_eq!(items.len(), 2);
    let first = items[0];
    assert_eq!(first.text_of("g:id"), "SKU-1");
    assert_eq!(
        first.text_of("g:title"),
        "Kávovar & čajník <Premium> „edice“"
    );
    assert_eq!(first.text_of("g:price"), "123.45 CZK");
    assert_eq!(first.text_of("g:availability"), "in_stock");
    assert_eq!(first.text_of("g:brand"), "Acme & Sons");
    assert_eq!(first.text_of("g:gtin"), "8591234567890");
    assert_eq!(first.text_of("g:condition"), "new");
    assert_eq!(first.text_of("g:product_type"), "Domov > Kuchyně");
    assert_eq!(
        first.text_of("g:image_link"),
        "https://example.test/img/sku-1-a.jpg"
    );
    assert_eq!(
        first.text_of("g:additional_image_link"),
        "https://example.test/img/sku-1-b.jpg?x=1&y=2"
    );
    assert_eq!(items[1].text_of("g:availability"), "out_of_stock");
    assert!(!items[1].has("g:brand"));
    assert!(!items[1].has("g:gtin"));
}

#[test]
fn xml_escaping_round_trips() {
    for portal in ["heureka", "google"] {
        let (xml, outcome) = run(&config_text(portal, ""), ESCAPING_CATALOG);
        assert_eq!(outcome.written, 1);
        assert!(!xml.contains("<čajník>"), "{xml}");
        assert!(xml.contains("&lt;čajník&gt;"), "{xml}");
        assert!(xml.contains("&amp;"), "{xml}");
        let root = parse(&xml);
        if root.name == "SHOP" {
            let item = root.children_named("SHOPITEM").next().unwrap();
            assert_eq!(item.text_of("PRODUCTNAME"), "Kávovar & <čajník>");
            assert_eq!(
                item.text_of("DESCRIPTION"),
                "Příliš \"žluťoučký\" kůň & <pes>"
            );
            assert_eq!(item.text_of("URL"), "https://example.test/p?a=1&b=2");
            assert_eq!(item.text_of("IMGURL"), "https://example.test/i.jpg?a=1&b=2");
            assert_eq!(item.text_of("CATEGORYTEXT"), "Dům & byt");
            assert_eq!(item.text_of("MANUFACTURER"), "Acme & Sons");
            let param = item.children_named("PARAM").next().unwrap();
            assert_eq!(param.text_of("PARAM_NAME"), "Vlastnost & klíč");
            assert_eq!(param.text_of("PARAM_VAL"), "hodnota <x> & \"y\"");
        } else {
            let item = root.child("channel").children_named("item").next().unwrap();
            assert_eq!(item.text_of("g:title"), "Kávovar & <čajník>");
            assert_eq!(item.text_of("g:link"), "https://example.test/p?a=1&b=2");
            assert_eq!(item.text_of("g:brand"), "Acme & Sons");
        }
    }
}

#[test]
fn description_truncates_on_character_boundary() {
    let description = "Příliš žluťoučký kůň úpěl ďábelské ódy";
    let catalog = catalog_with(serde_json::json!([{
        "id": "SKU-1",
        "title": "Kávovar",
        "description": description,
        "price_minor": 12345,
        "currency": "CZK",
        "availability": "in_stock",
        "url": "https://example.test/p/sku-1",
        "image_urls": ["https://example.test/i/sku-1.jpg"],
        "category_path": []
    }]));
    let expected: String = description.chars().take(5).collect();
    assert_eq!(expected, "Příli");
    for portal in ["heureka", "google"] {
        let (xml, outcome) = run(
            &config_text(portal, "max_description_chars = 5\n"),
            &catalog,
        );
        assert_eq!(outcome.written, 1);
        let root = parse(&xml);
        let element = if root.name == "SHOP" {
            root.children_named("SHOPITEM").next().unwrap()
        } else {
            root.child("channel").children_named("item").next().unwrap()
        };
        let tag = if root.name == "SHOP" {
            "DESCRIPTION"
        } else {
            "g:description"
        };
        let text = element.text_of(tag);
        assert_eq!(text, expected);
        assert_eq!(text.chars().count(), 5);
        assert!(!text.ends_with('\u{feff}'));
    }
}

#[test]
fn price_formats_minor_units() {
    assert_eq!(format_price_minor(0), "0.00");
    assert_eq!(format_price_minor(5), "0.05");
    assert_eq!(format_price_minor(100), "1.00");
    assert_eq!(format_price_minor(12345), "123.45");
    assert_eq!(format_price_minor(-250), "-2.50");
    assert_eq!(format_price_minor(-5), "-0.05");
}

#[test]
fn availability_mapping_override_applies() {
    let config = config_text(
        "heureka",
        "\n[availability]\nin_stock = \"SKLADEM\"\npreorder = \"PREDPRODEJ\"\n",
    );
    let (xml, _) = run(&config, CATALOG);
    let root = parse(&xml);
    let items: Vec<&Node> = root.children_named("SHOPITEM").collect();
    assert_eq!(items[0].text_of("AVAILABILITY"), "SKLADEM");
    assert_eq!(items[1].text_of("AVAILABILITY"), "1");
}

#[test]
fn category_map_rewrites_longest_prefix() {
    let catalog = catalog_with(serde_json::json!([{
        "id": "SKU-1",
        "title": "Hrnec",
        "description": "Velký hrnec.",
        "price_minor": 45900,
        "currency": "CZK",
        "availability": "in_stock",
        "url": "https://example.test/p/hrnec",
        "image_urls": ["https://example.test/i/hrnec.jpg"],
        "category_path": ["Domov", "Kuchyně", "Hrnce"]
    }]));
    let extra = "\n[category_map]\n\"Domov\" = \"Domácí potřeby\"\n\"Domov > Kuchyně\" = \"Kuchyně a jídlo\"\n";
    let (xml, _) = run(&config_text("heureka", extra), &catalog);
    let root = parse(&xml);
    let item = root.children_named("SHOPITEM").next().unwrap();
    assert_eq!(item.text_of("CATEGORYTEXT"), "Kuchyně a jídlo | Hrnce");
    let (xml, _) = run(&config_text("google", extra), &catalog);
    let rss = parse(&xml);
    let google_item = rss.child("channel").children_named("item").next().unwrap();
    assert_eq!(
        google_item.text_of("g:product_type"),
        "Kuchyně a jídlo > Hrnce"
    );
}

#[test]
fn invalid_products_are_skipped_with_reasons() {
    let mut no_description = valid_product("SKU-NODESC");
    no_description
        .as_object_mut()
        .unwrap()
        .remove("description");
    let mut bad_url = valid_product("SKU-BADURL");
    bad_url["url"] = serde_json::json!("not-a-url");
    let mut no_images = valid_product("SKU-NOIMG");
    no_images["image_urls"] = serde_json::json!([]);
    let mut usd = valid_product("SKU-USD");
    usd["currency"] = serde_json::json!("USD");
    let catalog = catalog_with(serde_json::json!([
        valid_product("SKU-OK"),
        no_description,
        bad_url,
        no_images,
        usd
    ]));
    let (xml, outcome) = run(&config_text("heureka", ""), &catalog);
    assert_eq!(outcome.written, 1);
    assert_eq!(outcome.skipped_count(), 4);
    assert_eq!(outcome.skipped[0].index, 1);
    assert_eq!(outcome.skipped[0].id.as_deref(), Some("SKU-NODESC"));
    assert!(
        outcome.skipped[0]
            .reasons
            .contains(&"missing_field:description".to_string())
    );
    assert_eq!(outcome.skipped[1].reasons, vec!["invalid_url:url"]);
    assert_eq!(outcome.skipped[2].reasons, vec!["missing_field:image"]);
    assert_eq!(outcome.skipped[3].reasons, vec!["currency_mismatch"]);
    let parsed: Outcome =
        serde_json::from_str(&serde_json::to_string(&outcome).unwrap()).expect("report round trip");
    assert_eq!(parsed, outcome);
    assert_eq!(
        parse(&xml).children_named("SHOPITEM").count(),
        1,
        "only the valid product is written"
    );
}

#[test]
fn required_fields_override_controls_validation() {
    let mut bare = valid_product("SKU-BARE");
    bare.as_object_mut().unwrap().remove("description");
    let catalog = catalog_with(serde_json::json!([bare]));
    let (_, outcome) = run(
        &config_text("heureka", "required_fields = [\"id\"]\n"),
        &catalog,
    );
    assert_eq!(outcome.written, 1);
    let (_, outcome) = run(
        &config_text("heureka", "required_fields = [\"id\", \"brand\"]\n"),
        &catalog,
    );
    assert_eq!(outcome.skipped[0].reasons, vec!["missing_field:brand"]);
}

#[test]
fn output_is_deterministic() {
    let (first, _) = run(&config_text("heureka", ""), CATALOG);
    let (second, _) = run(&config_text("heureka", ""), CATALOG);
    assert_eq!(first.as_bytes(), second.as_bytes());
    let (first, _) = run(&config_text("google", ""), CATALOG);
    let (second, _) = run(&config_text("google", ""), CATALOG);
    assert_eq!(first.as_bytes(), second.as_bytes());
}

#[test]
fn config_validation_rejects_bad_values() {
    assert!(
        Config::from_toml_str("portal = \"heureka\"\ncurrency = \"EUR\"\n").is_err(),
        "heureka accepts CZK only"
    );
    assert!(Config::from_toml_str("portal = \"google\"\ncurrency = \"EUR\"\n").is_ok());
    assert!(
        Config::from_toml_str(
            "portal = \"heureka\"\ncurrency = \"CZK\"\nmax_description_chars = 0\n"
        )
        .is_err()
    );
    assert!(
        Config::from_toml_str(
            "portal = \"heureka\"\ncurrency = \"CZK\"\n[availability]\nin_stock = \"\"\n"
        )
        .is_err()
    );
    assert!(
        Config::from_toml_str(
            "portal = \"heureka\"\ncurrency = \"CZK\"\n[availability]\nunknown = \"0\"\n"
        )
        .is_err()
    );
    assert!(
        Config::from_toml_str(
            "portal = \"heureka\"\ncurrency = \"CZK\"\nrequired_fields = [\"nope\"]\n"
        )
        .is_err()
    );
    assert!(Config::from_toml_str("portal = \"nope\"\ncurrency = \"CZK\"\n").is_err());
    assert!(
        Config::from_toml_str("portal = \"heureka\"\ncurrency = \"CZK\"\nextra = true\n").is_err()
    );
    assert!(Config::from_toml_str("portal = \"heureka\"\ncurrency = \"czk\"\n").is_ok());
    assert!(
        Config::from_toml_str(
            "portal = \"heureka\"\ncurrency = \"CZK\"\n[category_map]\n\"Domov > \" = \"X\"\n"
        )
        .is_err()
    );
}

#[test]
fn shop_must_appear_before_products() {
    let config = Config::from_toml_str(&config_text("heureka", "")).unwrap();
    let catalog = r#"{"products":[],"shop":{"name":"Example Shop","url":"https://example.test","email":"shop@example.test"}}"#;
    let error = generate(&config, catalog.as_bytes(), &mut Vec::new()).unwrap_err();
    let message = format!("{error:#}");
    assert!(message.contains("must appear before"), "{message}");
}

#[test]
fn malformed_catalog_is_rejected() {
    let config = Config::from_toml_str(&config_text("heureka", "")).unwrap();
    assert!(generate(&config, "{ not json".as_bytes(), &mut Vec::new()).is_err());
    assert!(generate(&config, "[]".as_bytes(), &mut Vec::new()).is_err());
}

#[test]
fn fixture_matches_library_catalog_model() {
    let catalog: Catalog = serde_json::from_str(CATALOG).expect("fixture parses");
    assert_eq!(catalog.shop.name, "Example Shop");
    assert_eq!(catalog.products.len(), 2);
    assert_eq!(catalog.products[0].weight_grams, Some(1500));
    assert_eq!(catalog.products[1].brand, None);
}
