#![allow(dead_code)]

use quick_xml::Reader;
use quick_xml::XmlVersion;
use quick_xml::escape::unescape;
use quick_xml::events::{BytesStart, Event};

#[derive(Debug, Default)]
pub struct Node {
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<Node>,
    pub text: String,
}

impl Node {
    #[track_caller]
    pub fn child(&self, name: &str) -> &Node {
        self.children
            .iter()
            .find(|node| node.name == name)
            .unwrap_or_else(|| panic!("missing child <{name}> in <{}>", self.name))
    }

    pub fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Node> {
        self.children.iter().filter(move |node| node.name == name)
    }

    pub fn has(&self, name: &str) -> bool {
        self.children.iter().any(|node| node.name == name)
    }

    #[track_caller]
    pub fn text_of(&self, name: &str) -> String {
        self.child(name).text.clone()
    }

    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }
}

#[track_caller]
pub fn parse(xml: &str) -> Node {
    let mut reader = Reader::from_str(xml);
    let mut stack: Vec<Node> = Vec::new();
    let mut root: Option<Node> = None;
    loop {
        match reader.read_event() {
            Ok(Event::Start(start)) => stack.push(node_from_start(&start)),
            Ok(Event::Empty(start)) => attach(&mut stack, &mut root, node_from_start(&start)),
            Ok(Event::Text(text)) => {
                let decoded = text.decode().expect("UTF-8 text");
                let value = unescape(&decoded).expect("valid XML entities");
                if let Some(current) = stack.last_mut() {
                    current.text.push_str(&value);
                }
            }
            Ok(Event::GeneralRef(reference)) => {
                let value = if reference.is_char_ref() {
                    reference
                        .resolve_char_ref()
                        .expect("valid character reference")
                        .unwrap_or_default()
                        .to_string()
                } else {
                    let name = reference.decode().expect("UTF-8 entity name");
                    quick_xml::escape::resolve_predefined_entity(&name)
                        .unwrap_or_default()
                        .to_string()
                };
                if let Some(current) = stack.last_mut() {
                    current.text.push_str(&value);
                }
            }
            Ok(Event::End(_)) => {
                let node = stack.pop().expect("balanced end tag");
                attach(&mut stack, &mut root, node);
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(error) => panic!("cannot parse XML: {error}"),
        }
    }
    assert!(stack.is_empty(), "unclosed XML elements");
    root.expect("XML document root")
}

fn node_from_start(start: &BytesStart<'_>) -> Node {
    let name = String::from_utf8_lossy(start.name().as_ref()).into_owned();
    let attrs = start
        .attributes()
        .map(|attribute| {
            let attribute = attribute.expect("valid attribute");
            let key = String::from_utf8_lossy(attribute.key.as_ref()).into_owned();
            let value = attribute
                .normalized_value(XmlVersion::Explicit1_0)
                .expect("valid attribute value")
                .into_owned();
            (key, value)
        })
        .collect();
    Node {
        name,
        attrs,
        children: Vec::new(),
        text: String::new(),
    }
}

fn attach(stack: &mut [Node], root: &mut Option<Node>, node: Node) {
    if let Some(parent) = stack.last_mut() {
        parent.children.push(node);
    } else {
        *root = Some(node);
    }
}
