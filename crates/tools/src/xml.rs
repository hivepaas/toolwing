//! `xml`: format and minify XML as `xmllint --format` and `xmllint
//! --noblanks` write it (libxml2 2.9.14): a document's well-formedness
//! checked (XML 1.0), its blank text dropped where libxml2 judges it
//! ignorable (`areBlanks`: by the DTD's element declarations, else by its
//! heuristic), and the rest written back as libxml2's serializer writes it:
//! the XML declaration always, attributes in double quotes, references
//! kept, the DTD's internal subset one declaration a line, and elements
//! indented only where they hold no text.
//!
//! What it does not do as xmllint does: read documents in encodings other
//! than UTF-8 (one in ISO-8859-1 or the like that is all ASCII is read), add
//! XHTML's `<meta>` to a document with an XHTML DOCTYPE, expand parameter
//! entities in the DTD, or stop at 256 levels: it reads 10,000, as the other
//! tools do (architecture.md).
//! Notations are written in the order they are declared; libxml2's order is
//! its hash table's, which changes run to run.

use toolwing_core::{
    Category, Cli, Compat, Ctx, Error, InputKind, Meta, Output, Param, ParamKind, Reference, Score,
    Span, Tool, Value, Values, en,
};

// ---- The tree ---------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
enum Node {
    Element(Element),
    Text(String),
    CData(String),
    Comment(String),
    Pi {
        target: String,
        content: Option<String>,
    },
    EntityRef(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Element {
    name: String,
    attrs: Vec<Attr>,
    children: Vec<Node>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Attr {
    name: String,
    value: Vec<AttrPart>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum AttrPart {
    Text(String),
    Ref(String),
}

/// An element declaration's content, as libxml2 holds it: a binary tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Occur {
    Once,
    Opt,
    Mult,
    Plus,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CType {
    PcData,
    Element,
    Seq,
    Or,
}

#[derive(Clone, Debug)]
struct CNode {
    ty: CType,
    name: String,
    occur: Occur,
    c1: Option<usize>,
    c2: Option<usize>,
    parent: Option<usize>,
}

#[derive(Clone, Debug, Default)]
struct Content {
    nodes: Vec<CNode>,
    root: Option<usize>,
}

#[derive(Clone, Debug)]
enum ElementType {
    Empty,
    Any,
    Mixed(Content),
    Children(Content),
}

#[derive(Clone, Debug)]
enum Decl {
    Element {
        name: String,
        ty: ElementType,
    },
    Attlist {
        elem: String,
        name: String,
        ty: String,
        default: String,
    },
    Entity {
        text: String,
    },
    Comment(String),
    Pi {
        target: String,
        content: Option<String>,
    },
}

#[derive(Clone, Debug)]
struct Entity {
    /// The replacement text, character references expanded.
    replacement: Option<String>,
    external: bool,
    unparsed: bool,
    checked: bool,
}

#[derive(Clone, Debug, Default)]
struct Doctype {
    name: String,
    public: Option<String>,
    system: Option<String>,
    decls: Vec<Decl>,
    notations: Vec<String>,
    /// Anything libxml2 keeps in its tables: then the subset is written.
    has_tables: bool,
}

#[derive(Clone, Debug, Default)]
struct Doc {
    version: Option<String>,
    encoding: Option<String>,
    standalone: Option<bool>,
    doctype: Option<Doctype>,
    /// The doctype's place among the top-level nodes.
    doctype_at: usize,
    top: Vec<Node>,
}

// ---- Characters ---------------------------------------------------------------

fn is_char(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\r' | '\u{20}'..='\u{D7FF}' | '\u{E000}'..='\u{FFFD}' | '\u{10000}'..)
}

fn is_blank(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | b'\r')
}

fn is_name_start(c: char) -> bool {
    matches!(c, ':' | 'A'..='Z' | '_' | 'a'..='z'
        | '\u{C0}'..='\u{D6}' | '\u{D8}'..='\u{F6}' | '\u{F8}'..='\u{2FF}'
        | '\u{370}'..='\u{37D}' | '\u{37F}'..='\u{1FFF}' | '\u{200C}'..='\u{200D}'
        | '\u{2070}'..='\u{218F}' | '\u{2C00}'..='\u{2FEF}' | '\u{3001}'..='\u{D7FF}'
        | '\u{F900}'..='\u{FDCF}' | '\u{FDF0}'..='\u{FFFD}' | '\u{10000}'..='\u{EFFFF}')
}

fn is_name_char(c: char) -> bool {
    is_name_start(c)
        || matches!(c, '-' | '.' | '0'..='9' | '\u{B7}' | '\u{300}'..='\u{36F}' | '\u{203F}'..='\u{2040}')
}

/// Deeper than this is refused, as the other tools refuse it.
const MAX_DEPTH: usize = 10_000;

// ---- The parser ---------------------------------------------------------------

struct Parser<'a> {
    s: &'a str,
    i: usize,
    /// Where `s` starts in the document, for errors in an entity's text.
    base: Option<usize>,
}

type Res<T> = Result<T, (usize, String)>;

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<u8> {
        self.s.as_bytes().get(self.i).copied()
    }

    fn at(&self, lit: &str) -> bool {
        self.s[self.i..].starts_with(lit)
    }

    fn eat(&mut self, lit: &str) -> bool {
        if self.at(lit) {
            self.i += lit.len();
            true
        } else {
            false
        }
    }

    fn err<T>(&self, msg: impl Into<String>) -> Res<T> {
        Err((self.base.unwrap_or(self.i), msg.into()))
    }

    fn expect(&mut self, lit: &str) -> Res<()> {
        if self.eat(lit) {
            Ok(())
        } else {
            self.err(format!("expected {lit}"))
        }
    }

    fn skip_blanks(&mut self) -> bool {
        let start = self.i;
        while self.peek().is_some_and(is_blank) {
            self.i += 1;
        }
        self.i > start
    }

    fn need_blanks(&mut self, what: &str) -> Res<()> {
        if self.skip_blanks() {
            Ok(())
        } else {
            self.err(format!("space required {what}"))
        }
    }

    fn name(&mut self) -> Res<String> {
        let rest = &self.s[self.i..];
        let mut chars = rest.char_indices();
        match chars.next() {
            Some((_, c)) if is_name_start(c) => {}
            _ => return self.err("a name was expected"),
        }
        let end = chars
            .find(|(_, c)| !is_name_char(*c))
            .map_or(rest.len(), |(i, _)| i);
        self.i += end;
        Ok(rest[..end].to_owned())
    }

    fn quoted(&mut self) -> Res<String> {
        let q = match self.peek() {
            Some(q @ (b'"' | b'\'')) => q,
            _ => return self.err("a quoted value was expected"),
        };
        self.i += 1;
        let rest = &self.s[self.i..];
        let end = rest
            .find(q as char)
            .ok_or(())
            .or_else(|_| self.err("a quote is not closed"))?;
        let value = &rest[..end];
        if let Some((at, c)) = value.char_indices().find(|(_, c)| !is_char(*c)) {
            self.i += at;
            return self.err(format!(
                "U+{:04X} is not a character XML allows",
                u32::from(c)
            ));
        }
        self.i += end + 1;
        Ok(value.to_owned())
    }

    /// `&#…;` after the `&`, as a character.
    fn char_ref(&mut self) -> Res<char> {
        let hex = self.eat("#x");
        if !hex {
            self.expect("#")?;
        }
        let rest = &self.s[self.i..];
        let end = rest
            .find(';')
            .ok_or(())
            .or_else(|_| self.err("a character reference has no ';'"))?;
        let digits = &rest[..end];
        let n = if hex {
            (!digits.is_empty() && digits.bytes().all(|b| b.is_ascii_hexdigit()))
                .then(|| u32::from_str_radix(digits, 16).ok())
                .flatten()
        } else {
            (!digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
                .then(|| digits.parse().ok())
                .flatten()
        };
        let c = n.and_then(char::from_u32).filter(|c| is_char(*c));
        match c {
            Some(c) => {
                self.i += end + 1;
                Ok(c)
            }
            None => self.err(format!(
                "&#{}{digits}; is not a character XML allows",
                if hex { "x" } else { "" }
            )),
        }
    }
}

struct Builder {
    doc: Doc,
    entities: std::collections::HashMap<String, Entity>,
    /// Element declarations: true for element content (blanks ignorable).
    elements: std::collections::HashMap<String, bool>,
    /// Attributes declared with a type other than CDATA.
    tokenized: std::collections::HashSet<(String, String)>,
    attlists: std::collections::HashSet<(String, String)>,
    external_subset: bool,
}

fn predefined(name: &str) -> Option<char> {
    Some(match name {
        "lt" => '<',
        "gt" => '>',
        "amp" => '&',
        "apos" => '\'',
        "quot" => '"',
        _ => return None,
    })
}

/// An element being read: its tree and its xml:space.
struct Open {
    el: Element,
    space: i8,
}

impl Builder {
    fn undeclared_is_error(&self) -> bool {
        self.doc.standalone == Some(true) || !self.external_subset
    }

    /// An entity's replacement text, checked well-formed once: as content,
    /// or for an attribute value, without `<`.
    fn check_entity(&mut self, name: &str, in_attribute: bool, depth: usize) -> Result<(), String> {
        if depth > 40 {
            return Err(format!("the entity {name} refers to itself"));
        }
        let Some(e) = self.entities.get(name).cloned() else {
            return Ok(());
        };
        if e.unparsed {
            return Err(format!("the unparsed entity {name} is referred to"));
        }
        if e.external {
            if in_attribute {
                return Err(format!("an attribute refers to the external entity {name}"));
            }
            return Ok(());
        }
        let text = e.replacement.unwrap_or_default();
        if in_attribute && text.contains('<') {
            return Err(format!("the entity {name} puts '<' in an attribute value"));
        }
        if e.checked && !in_attribute {
            return Ok(());
        }
        // References within it, each checked the same way.
        let mut p = Parser {
            s: &text,
            i: 0,
            base: Some(0),
        };
        if in_attribute {
            while let Some(at) = p.s[p.i..].find('&') {
                p.i += at + 1;
                if p.at("#") {
                    p.char_ref().map_err(|e| e.1)?;
                } else {
                    let n = p.name().map_err(|e| e.1)?;
                    p.expect(";").map_err(|e| e.1)?;
                    if predefined(&n).is_none() {
                        self.reference(&n, true, depth + 1)?;
                    }
                }
            }
            return Ok(());
        }
        let mut stack = vec![Open {
            el: Element {
                name: String::new(),
                attrs: Vec::new(),
                children: Vec::new(),
            },
            space: 1,
        }];
        self.content(&mut p, &mut stack, depth + 1, true)
            .map_err(|e| e.1)?;
        if stack.len() != 1 {
            return Err(format!("the entity {name} leaves an element open"));
        }
        if let Some(e) = self.entities.get_mut(name) {
            e.checked = true;
        }
        Ok(())
    }

    /// A general entity referred to: declared, or allowed to be undeclared.
    fn reference(&mut self, name: &str, in_attribute: bool, depth: usize) -> Result<bool, String> {
        if self.entities.contains_key(name) {
            self.check_entity(name, in_attribute, depth)?;
            Ok(true)
        } else if self.undeclared_is_error() {
            Err(format!("Entity '{name}' not defined"))
        } else {
            Ok(false)
        }
    }

    fn append(stack: &mut [Open], node: Node) {
        let Some(top) = stack.last_mut() else { return };
        if let (Node::Text(t), Some(Node::Text(last))) = (&node, top.el.children.last_mut()) {
            last.push_str(t);
            return;
        }
        top.el.children.push(node);
    }

    /// libxml2's `areBlanks`: whether blank text before `next` is dropped.
    fn are_blanks(&self, top: &Open, next: &str) -> bool {
        if top.space == 1 || top.space == -2 {
            return false;
        }
        if let Some(element_content) = self.elements.get(&top.el.name) {
            return *element_content;
        }
        let raw = next.as_bytes().first().copied();
        if raw != Some(b'<') {
            return false;
        }
        let children = &top.el.children;
        if children.is_empty() && next.starts_with("</") {
            return false;
        }
        match (children.first(), children.last()) {
            (_, None) => true,
            (_, Some(Node::Text(_))) => false,
            (Some(Node::Text(_)), _) => false,
            _ => true,
        }
    }

    /// Content until the element at the bottom of the stack ends (or, for
    /// an entity's text, until the text does).
    fn content(
        &mut self,
        p: &mut Parser,
        stack: &mut Vec<Open>,
        depth: usize,
        fragment: bool,
    ) -> Res<()> {
        let floor = stack.len();
        loop {
            if fragment && p.i >= p.s.len() {
                return Ok(());
            }
            let Some(b) = p.peek() else {
                return p.err(format!(
                    "the document ends inside <{}>",
                    stack.last().map(|o| o.el.name.as_str()).unwrap_or("")
                ));
            };
            if b == b'<' {
                if p.at("</") {
                    if stack.len() == floor && fragment {
                        return p.err("an entity's text closes an element it did not open");
                    }
                    p.i += 2;
                    let name = p.name()?;
                    p.skip_blanks();
                    p.expect(">")?;
                    let open = stack.pop().map(|o| o.el);
                    let Some(el) = open else {
                        return p.err("an end tag has no start");
                    };
                    if el.name != name {
                        return p.err(format!(
                            "Opening and ending tag mismatch: {} and {name}",
                            el.name
                        ));
                    }
                    if stack.len() < floor || stack.is_empty() {
                        stack.push(Open { el, space: 0 });
                        return Ok(());
                    }
                    Self::append(stack, Node::Element(el));
                    if stack.len() < floor {
                        return Ok(());
                    }
                    continue;
                }
                if p.at("<!--") {
                    let c = comment(p)?;
                    Self::append(stack, Node::Comment(c));
                    continue;
                }
                if p.at("<![CDATA[") {
                    p.i += 9;
                    let rest = &p.s[p.i..];
                    let end = rest
                        .find("]]>")
                        .ok_or(())
                        .or_else(|_| p.err("a CDATA section is not closed"))?;
                    let text = rest[..end].to_owned();
                    p.i += end + 3;
                    Self::append(stack, Node::CData(text));
                    continue;
                }
                if p.at("<?") {
                    let (target, content) = pi(p)?;
                    Self::append(stack, Node::Pi { target, content });
                    continue;
                }
                if p.at("<!") {
                    return p.err("markup that is not allowed here");
                }
                // A start tag.
                p.i += 1;
                let el = self.start_tag(p)?;
                if stack.len() >= MAX_DEPTH {
                    return p.err(format!("deeper than {MAX_DEPTH} levels"));
                }
                let inherited = stack
                    .last()
                    .map_or(-1, |o| if o.space == -2 { -1 } else { o.space });
                let space = match el.attrs.iter().find(|a| a.name == "xml:space") {
                    Some(a) if attr_text(a) == "default" => 0,
                    Some(a) if attr_text(a) == "preserve" => 1,
                    _ => inherited,
                };
                if p.eat("/>") {
                    if stack.is_empty() {
                        stack.push(Open { el, space });
                        return Ok(());
                    }
                    Self::append(stack, Node::Element(el));
                } else {
                    p.expect(">")?;
                    stack.push(Open { el, space });
                }
                continue;
            }
            if b == b'&' {
                p.i += 1;
                if p.at("#") {
                    let c = p.char_ref()?;
                    Self::append(stack, Node::Text(c.to_string()));
                    continue;
                }
                let name = p.name()?;
                p.expect(";")?;
                if let Some(c) = predefined(&name) {
                    Self::append(stack, Node::Text(c.to_string()));
                    continue;
                }
                let at = p.i;
                self.reference(&name, false, depth)
                    .map_err(|m| (p.base.unwrap_or(at), m))?;
                Self::append(stack, Node::EntityRef(name));
                continue;
            }
            // Character data, to the next markup or reference.
            let rest = &p.s[p.i..];
            let end = rest.find(['<', '&']).unwrap_or(rest.len());
            let text = &rest[..end];
            if let Some(at) = text.find("]]>") {
                p.i += at;
                return p.err("']]>' is not allowed in text");
            }
            if let Some((at, c)) = text.char_indices().find(|(_, c)| !is_char(*c)) {
                p.i += at;
                return p.err(format!(
                    "U+{:04X} is not a character XML allows",
                    u32::from(c)
                ));
            }
            if stack.is_empty() {
                return p.err("text outside the root element");
            }
            p.i += end;
            self.text(stack, text, &p.s[p.i..]);
        }
    }

    /// Character data, in the pieces libxml2's `xmlParseCharData` reads it
    /// in: the ASCII run first, then what follows its first other character.
    /// A piece that starts blank is dropped if `areBlanks` says so; one kept
    /// marks its element (xml:space -2) so that its later blanks are kept
    /// too, as is the second piece always.
    fn text(&self, stack: &mut [Open], text: &str, after: &str) {
        let split = text.find(|c: char| !c.is_ascii()).unwrap_or(text.len());
        let (ascii, rest) = text.split_at(split);
        let mark = |stack: &mut [Open]| {
            if let Some(top) = stack.last_mut()
                && top.space == -1
            {
                top.space = -2;
            }
        };
        if !ascii.is_empty() {
            if ascii.as_bytes().first().copied().is_some_and(is_blank) {
                let next = if rest.is_empty() { after } else { rest };
                let drop = ascii.bytes().all(is_blank)
                    && stack.last().is_some_and(|top| self.are_blanks(top, next));
                if !drop {
                    Self::append(stack, Node::Text(ascii.to_owned()));
                    mark(stack);
                }
            } else {
                Self::append(stack, Node::Text(ascii.to_owned()));
            }
        }
        if !rest.is_empty() {
            Self::append(stack, Node::Text(rest.to_owned()));
            mark(stack);
        }
    }

    fn start_tag(&mut self, p: &mut Parser) -> Res<Element> {
        let name = p.name()?;
        let mut attrs: Vec<Attr> = Vec::new();
        loop {
            let spaced = p.skip_blanks();
            if p.at("/>") || p.at(">") {
                break;
            }
            if !spaced {
                return p.err("attributes must be separated by space");
            }
            let an = p.name()?;
            p.skip_blanks();
            p.expect("=")?;
            p.skip_blanks();
            let q = match p.peek() {
                Some(q @ (b'"' | b'\'')) => q,
                _ => return p.err(format!("AttValue: \" or ' expected for {an}")),
            };
            p.i += 1;
            let mut parts: Vec<AttrPart> = Vec::new();
            let mut text = String::new();
            loop {
                let Some(c) = p.s[p.i..].chars().next() else {
                    return p.err("an attribute value is not closed");
                };
                if c as u32 == u32::from(q) {
                    p.i += 1;
                    break;
                }
                match c {
                    '<' => return p.err(format!("'<' in the value of {an}")),
                    '&' => {
                        p.i += 1;
                        if p.at("#") {
                            text.push(p.char_ref()?);
                            continue;
                        }
                        let rn = p.name()?;
                        p.expect(";")?;
                        if let Some(c) = predefined(&rn) {
                            text.push(c);
                            continue;
                        }
                        let at = p.i;
                        // Declared, or undeclared where an external subset
                        // may declare it: kept as the reference.
                        self.reference(&rn, true, 0).map_err(|m| (at, m))?;
                        if !text.is_empty() {
                            parts.push(AttrPart::Text(std::mem::take(&mut text)));
                        }
                        parts.push(AttrPart::Ref(rn));
                    }
                    '\t' | '\n' | '\r' => {
                        text.push(' ');
                        p.i += 1;
                    }
                    c if !is_char(c) => {
                        return p.err(format!(
                            "U+{:04X} is not a character XML allows",
                            u32::from(c)
                        ));
                    }
                    c => {
                        text.push(c);
                        p.i += c.len_utf8();
                    }
                }
            }
            if !text.is_empty() || parts.is_empty() {
                parts.push(AttrPart::Text(text));
            }
            if self.tokenized.contains(&(name.clone(), an.clone())) {
                parts = parts
                    .into_iter()
                    .map(|part| match part {
                        AttrPart::Text(t) => AttrPart::Text(
                            t.split(' ')
                                .filter(|w| !w.is_empty())
                                .collect::<Vec<_>>()
                                .join(" "),
                        ),
                        r => r,
                    })
                    .collect();
            }
            if attrs.iter().any(|a| a.name == an) {
                return p.err(format!("Attribute {an} redefined"));
            }
            attrs.push(Attr {
                name: an,
                value: parts,
            });
        }
        Ok(Element {
            name,
            attrs,
            children: Vec::new(),
        })
    }
}

fn attr_text(a: &Attr) -> String {
    a.value
        .iter()
        .map(|p| match p {
            AttrPart::Text(t) => t.clone(),
            AttrPart::Ref(r) => format!("&{r};"),
        })
        .collect()
}

fn comment(p: &mut Parser) -> Res<String> {
    p.i += 4;
    let rest = &p.s[p.i..];
    let end = rest
        .find("--")
        .ok_or(())
        .or_else(|_| p.err("a comment is not closed"))?;
    if !rest[end..].starts_with("-->") {
        p.i += end;
        return p.err("'--' is not allowed in a comment");
    }
    let text = &rest[..end];
    if let Some(c) = text.chars().find(|c| !is_char(*c)) {
        return p.err(format!(
            "U+{:04X} is not a character XML allows",
            u32::from(c)
        ));
    }
    p.i += end + 3;
    Ok(text.to_owned())
}

fn pi(p: &mut Parser) -> Res<(String, Option<String>)> {
    p.i += 2;
    let target = p.name()?;
    if target.eq_ignore_ascii_case("xml") {
        return p.err("XML declaration allowed only at the start of the document");
    }
    if p.eat("?>") {
        return Ok((target, None));
    }
    if !p.skip_blanks() {
        return p.err("space required after the target of a processing instruction");
    }
    let rest = &p.s[p.i..];
    let end = rest
        .find("?>")
        .ok_or(())
        .or_else(|_| p.err("a processing instruction is not closed"))?;
    let content = rest[..end].to_owned();
    p.i += end + 2;
    Ok((target, Some(content)))
}

// ---- The DTD's internal subset ----------------------------------------------

impl Builder {
    fn external_id(
        &mut self,
        p: &mut Parser,
        notation: bool,
    ) -> Res<(Option<String>, Option<String>)> {
        if p.eat("SYSTEM") {
            p.need_blanks("after SYSTEM")?;
            return Ok((None, Some(p.quoted()?)));
        }
        if p.eat("PUBLIC") {
            p.need_blanks("after PUBLIC")?;
            let public = p.quoted()?;
            if let Some(c) = public
                .chars()
                .find(|c| !(c.is_ascii_alphanumeric() || " \r\n-'()+,./:=?;!*#@$_%".contains(*c)))
            {
                return p.err(format!("{c:?} is not allowed in a public identifier"));
            }
            let save = p.i;
            let spaced = p.skip_blanks();
            if spaced && matches!(p.peek(), Some(b'"' | b'\'')) {
                return Ok((Some(public), Some(p.quoted()?)));
            }
            if notation {
                p.i = save;
                return Ok((Some(public), None));
            }
            return p.err("a system literal was expected after the public identifier");
        }
        Ok((None, None))
    }

    fn internal_subset(&mut self, p: &mut Parser, dt: &mut Doctype) -> Res<()> {
        loop {
            p.skip_blanks();
            if p.eat("]") {
                return Ok(());
            }
            if p.at("%") {
                return p.err("tw xml does not expand parameter entities in the DTD");
            }
            if p.at("<!--") {
                let c = comment(p)?;
                dt.decls.push(Decl::Comment(c));
                continue;
            }
            if p.at("<?") {
                let (target, content) = pi(p)?;
                dt.decls.push(Decl::Pi { target, content });
                continue;
            }
            if p.eat("<!ELEMENT") {
                p.need_blanks("after <!ELEMENT")?;
                let name = p.name()?;
                p.need_blanks("after the element's name")?;
                let ty = if p.eat("EMPTY") {
                    ElementType::Empty
                } else if p.eat("ANY") {
                    ElementType::Any
                } else if p.eat("(") {
                    p.skip_blanks();
                    if p.at("#PCDATA") {
                        ElementType::Mixed(mixed(p)?)
                    } else {
                        let mut c = Content::default();
                        let root = children(p, &mut c, 0)?;
                        c.root = Some(root);
                        ElementType::Children(c)
                    }
                } else {
                    return p.err("an element's content is EMPTY, ANY or (…)");
                };
                p.skip_blanks();
                p.expect(">")?;
                dt.has_tables = true;
                if !self.elements.contains_key(&name) {
                    self.elements
                        .insert(name.clone(), matches!(ty, ElementType::Children(_)));
                    dt.decls.push(Decl::Element { name, ty });
                }
                continue;
            }
            if p.eat("<!ATTLIST") {
                p.need_blanks("after <!ATTLIST")?;
                let elem = p.name()?;
                loop {
                    let spaced = p.skip_blanks();
                    if p.eat(">") {
                        break;
                    }
                    if !spaced {
                        return p.err("space required in <!ATTLIST");
                    }
                    let name = p.name()?;
                    p.need_blanks("after the attribute's name")?;
                    let ty = if p.eat("(") {
                        format!("({})", names_list(p, false)?)
                    } else if p.eat("NOTATION") {
                        p.need_blanks("after NOTATION")?;
                        p.expect("(")?;
                        format!("NOTATION ({})", names_list(p, true)?)
                    } else {
                        let t = p.name()?;
                        if !matches!(
                            t.as_str(),
                            "CDATA"
                                | "ID"
                                | "IDREF"
                                | "IDREFS"
                                | "ENTITY"
                                | "ENTITIES"
                                | "NMTOKEN"
                                | "NMTOKENS"
                        ) {
                            return p.err(format!("{t} is not an attribute type"));
                        }
                        t
                    };
                    p.need_blanks("after the attribute's type")?;
                    let default = if p.eat("#REQUIRED") {
                        "#REQUIRED".to_owned()
                    } else if p.eat("#IMPLIED") {
                        "#IMPLIED".to_owned()
                    } else {
                        let fixed = p.eat("#FIXED");
                        if fixed {
                            p.need_blanks("after #FIXED")?;
                        }
                        let v = default_value(p)?;
                        let q = quote(&v);
                        if fixed { format!("#FIXED {q}") } else { q }
                    };
                    dt.has_tables = true;
                    let key = (elem.clone(), name.clone());
                    if self.attlists.insert(key.clone()) {
                        if ty != "CDATA" {
                            self.tokenized.insert(key);
                        }
                        dt.decls.push(Decl::Attlist {
                            elem: elem.clone(),
                            name,
                            ty,
                            default,
                        });
                    }
                }
                continue;
            }
            if p.eat("<!ENTITY") {
                p.need_blanks("after <!ENTITY")?;
                let parameter = p.eat("%");
                if parameter {
                    p.need_blanks("after %")?;
                }
                let name = p.name()?;
                p.need_blanks("after the entity's name")?;
                let mut text = format!("<!ENTITY {}{name}", if parameter { "% " } else { "" });
                let mut entity = Entity {
                    replacement: None,
                    external: false,
                    unparsed: false,
                    checked: false,
                };
                if matches!(p.peek(), Some(b'"' | b'\'')) {
                    let (orig, replacement) = entity_value(p)?;
                    text.push(' ');
                    text.push_str(&quote(&orig));
                    entity.replacement = Some(replacement);
                } else {
                    let (public, system) = self.external_id(p, false)?;
                    let Some(system) = system else {
                        return p.err("an entity is a quoted value, SYSTEM or PUBLIC");
                    };
                    match public {
                        Some(pb) => {
                            text.push_str(&format!(" PUBLIC {} {}", quote(&pb), quote(&system)))
                        }
                        None => text.push_str(&format!(" SYSTEM {}", quote(&system))),
                    }
                    entity.external = true;
                    let save = p.i;
                    if p.skip_blanks() && p.eat("NDATA") {
                        if parameter {
                            return p.err("a parameter entity cannot be NDATA");
                        }
                        p.need_blanks("after NDATA")?;
                        let n = p.name()?;
                        text.push_str(&format!(" NDATA {n}"));
                        entity.unparsed = true;
                    } else {
                        p.i = save;
                    }
                }
                p.skip_blanks();
                p.expect(">")?;
                text.push('>');
                dt.has_tables = true;
                let key = if parameter {
                    format!("%{name}")
                } else {
                    name.clone()
                };
                // The first declaration counts; a predefined one is ignored.
                if parameter || predefined(&name).is_none() {
                    let checked = parameter || entity.checked;
                    if let std::collections::hash_map::Entry::Vacant(slot) =
                        self.entities.entry(key)
                    {
                        slot.insert(Entity { checked, ..entity });
                        dt.decls.push(Decl::Entity { text });
                    }
                }
                continue;
            }
            if p.eat("<!NOTATION") {
                p.need_blanks("after <!NOTATION")?;
                let name = p.name()?;
                p.need_blanks("after the notation's name")?;
                let (public, system) = self.external_id(p, true)?;
                p.skip_blanks();
                p.expect(">")?;
                let mut text = format!("<!NOTATION {name}");
                match (public, system) {
                    (Some(pb), sys) => {
                        text.push_str(&format!(" PUBLIC {}", quote(&pb)));
                        if let Some(s) = sys {
                            text.push_str(&format!(" {}", quote(&s)));
                        }
                    }
                    (None, Some(s)) => text.push_str(&format!(" SYSTEM {}", quote(&s))),
                    (None, None) => return p.err("a notation is SYSTEM or PUBLIC"),
                }
                text.push_str(" >");
                dt.has_tables = true;
                dt.notations.push(text);
                continue;
            }
            return p.err("a markup declaration was expected in the DTD");
        }
    }
}

fn names_list(p: &mut Parser, names: bool) -> Res<String> {
    let mut items = Vec::new();
    loop {
        p.skip_blanks();
        let item = if names {
            p.name()?
        } else {
            let rest = &p.s[p.i..];
            let end = rest.find(|c: char| !is_name_char(c)).unwrap_or(rest.len());
            if end == 0 {
                return p.err("a name token was expected");
            }
            p.i += end;
            rest[..end].to_owned()
        };
        items.push(item);
        p.skip_blanks();
        if p.eat(")") {
            return Ok(items.join(" | "));
        }
        p.expect("|")?;
    }
}

fn default_value(p: &mut Parser) -> Res<String> {
    let raw = p.quoted()?;
    if raw.contains('<') {
        return p.err("'<' in an attribute's default value");
    }
    let mut out = String::new();
    let mut sub = Parser {
        s: &raw,
        i: 0,
        base: Some(p.i),
    };
    while let Some(c) = sub.s[sub.i..].chars().next() {
        // References checked, and kept as written: libxml2 writes them
        // expanded, which can make its output not XML (a lone &).
        if c == '&' {
            let start = sub.i;
            sub.i += 1;
            if sub.at("#") {
                sub.char_ref()?;
            } else {
                sub.name()?;
                sub.expect(";")?;
            }
            out.push_str(&sub.s[start..sub.i]);
            continue;
        }
        out.push(if matches!(c, '\t' | '\n' | '\r') {
            ' '
        } else {
            c
        });
        sub.i += c.len_utf8();
    }
    Ok(out)
}

/// An entity value: as written, and with character references expanded.
fn entity_value(p: &mut Parser) -> Res<(String, String)> {
    let at = p.i;
    let orig = p.quoted()?;
    let mut out = String::new();
    let mut sub = Parser {
        s: &orig,
        i: 0,
        base: Some(at),
    };
    while let Some(c) = sub.s[sub.i..].chars().next() {
        match c {
            '%' => return sub.err("PEReferences forbidden in internal subset"),
            '&' => {
                sub.i += 1;
                if sub.at("#") {
                    out.push(sub.char_ref()?);
                } else {
                    let n = sub.name()?;
                    sub.expect(";")?;
                    out.push_str(&format!("&{n};"));
                }
            }
            c if !is_char(c) => {
                return sub.err(format!(
                    "U+{:04X} is not a character XML allows",
                    u32::from(c)
                ));
            }
            c => {
                out.push(c);
                sub.i += c.len_utf8();
            }
        }
    }
    Ok((orig, out))
}

/// libxml2's `xmlBufWriteQuotedString`: in double quotes, or single when
/// the text has double ones, or double with `&quot;` when it has both.
fn quote(s: &str) -> String {
    if s.contains('"') {
        if s.contains('\'') {
            format!("\"{}\"", s.replace('"', "&quot;"))
        } else {
            format!("'{s}'")
        }
    } else {
        format!("\"{s}\"")
    }
}

fn occur(p: &mut Parser) -> Occur {
    match p.peek() {
        Some(b'?') => {
            p.i += 1;
            Occur::Opt
        }
        Some(b'*') => {
            p.i += 1;
            Occur::Mult
        }
        Some(b'+') => {
            p.i += 1;
            Occur::Plus
        }
        _ => Occur::Once,
    }
}

fn new_node(c: &mut Content, ty: CType, name: &str) -> usize {
    c.nodes.push(CNode {
        ty,
        name: name.to_owned(),
        occur: Occur::Once,
        c1: None,
        c2: None,
        parent: None,
    });
    c.nodes.len() - 1
}

/// `(#PCDATA | a | b)*` after the `(`, as libxml2's
/// `xmlParseElementMixedContentDecl` builds it.
fn mixed(p: &mut Parser) -> Res<Content> {
    p.expect("#PCDATA")?;
    let mut c = Content::default();
    let mut ret = new_node(&mut c, CType::PcData, "");
    let mut cur = ret;
    let mut elem: Option<String> = None;
    p.skip_blanks();
    while p.eat("|") {
        p.skip_blanks();
        match elem.take() {
            None => {
                let n = new_node(&mut c, CType::Or, "");
                c.nodes[n].c1 = Some(cur);
                c.nodes[cur].parent = Some(n);
                ret = n;
                cur = n;
            }
            Some(e) => {
                let n = new_node(&mut c, CType::Or, "");
                let leaf = new_node(&mut c, CType::Element, &e);
                c.nodes[n].c1 = Some(leaf);
                c.nodes[leaf].parent = Some(n);
                c.nodes[cur].c2 = Some(n);
                c.nodes[n].parent = Some(cur);
                cur = n;
            }
        }
        elem = Some(p.name()?);
        p.skip_blanks();
    }
    if p.eat(")*") {
        if let Some(e) = elem {
            let leaf = new_node(&mut c, CType::Element, &e);
            c.nodes[cur].c2 = Some(leaf);
            c.nodes[leaf].parent = Some(cur);
        }
        c.nodes[ret].occur = Occur::Mult;
    } else if p.eat(")") {
        if elem.is_some() {
            return p.err("(#PCDATA | …) must end in )*");
        }
    } else {
        return p.err("( #PCDATA … ) is not closed");
    }
    c.root = Some(ret);
    Ok(c)
}

/// The children content after a `(`, as libxml2's
/// `xmlParseElementChildrenContentDeclPriv` builds it, normalisations and
/// all.
fn children(p: &mut Parser, c: &mut Content, depth: usize) -> Res<usize> {
    if depth > 128 {
        return p.err("an element declaration nests too deep");
    }
    p.skip_blanks();
    let mut ret;
    let mut cur;
    if p.eat("(") {
        p.skip_blanks();
        ret = children(p, c, depth + 1)?;
        cur = ret;
        p.skip_blanks();
    } else {
        let name = p.name()?;
        ret = new_node(c, CType::Element, &name);
        cur = ret;
        c.nodes[cur].occur = occur(p);
    }
    p.skip_blanks();
    let mut ty: Option<u8> = None;
    let mut last: Option<usize> = None;
    while p.peek() != Some(b')') {
        let sep = match p.peek() {
            Some(s @ (b',' | b'|')) => s,
            _ => return p.err("an element declaration's content is not closed"),
        };
        match ty {
            None => ty = Some(sep),
            Some(t) if t != sep => return p.err(format!("'{}' expected", t as char)),
            _ => {}
        }
        p.i += 1;
        let op = new_node(c, if sep == b',' { CType::Seq } else { CType::Or }, "");
        match last {
            None => {
                c.nodes[op].c1 = Some(ret);
                c.nodes[ret].parent = Some(op);
                ret = op;
                cur = op;
            }
            Some(l) => {
                c.nodes[cur].c2 = Some(op);
                c.nodes[op].parent = Some(cur);
                c.nodes[op].c1 = Some(l);
                c.nodes[l].parent = Some(op);
                cur = op;
            }
        }
        p.skip_blanks();
        if p.eat("(") {
            p.skip_blanks();
            last = Some(children(p, c, depth + 1)?);
            p.skip_blanks();
        } else {
            let name = p.name()?;
            let l = new_node(c, CType::Element, &name);
            c.nodes[l].occur = occur(p);
            last = Some(l);
        }
        p.skip_blanks();
    }
    if let Some(l) = last {
        c.nodes[cur].c2 = Some(l);
        c.nodes[l].parent = Some(cur);
    }
    p.i += 1;
    let once_children = |c: &mut Content, mut at: Option<usize>| -> bool {
        let mut found = false;
        while let Some(n) = at {
            if c.nodes[n].ty != CType::Or {
                break;
            }
            for child in [c.nodes[n].c1, c.nodes[n].c2].into_iter().flatten() {
                if matches!(c.nodes[child].occur, Occur::Opt | Occur::Mult) {
                    c.nodes[child].occur = Occur::Once;
                    found = true;
                }
            }
            at = c.nodes[n].c2;
        }
        found
    };
    match p.peek() {
        Some(b'?') => {
            c.nodes[ret].occur = if matches!(c.nodes[ret].occur, Occur::Plus | Occur::Mult) {
                Occur::Mult
            } else {
                Occur::Opt
            };
            p.i += 1;
        }
        Some(b'*') => {
            c.nodes[ret].occur = Occur::Mult;
            once_children(c, Some(ret));
            p.i += 1;
        }
        Some(b'+') => {
            c.nodes[ret].occur = if matches!(c.nodes[ret].occur, Occur::Opt | Occur::Mult) {
                Occur::Mult
            } else {
                Occur::Plus
            };
            // libxml2 walks from its last group here, not the first.
            if once_children(c, Some(cur)) {
                c.nodes[ret].occur = Occur::Mult;
            }
            p.i += 1;
        }
        _ => {}
    }
    Ok(ret)
}

fn occur_text(o: Occur) -> &'static str {
    match o {
        Occur::Once => "",
        Occur::Opt => "?",
        Occur::Mult => "*",
        Occur::Plus => "+",
    }
}

/// libxml2's `xmlDumpElementContent`.
fn dump_content(c: &Content, out: &mut String) {
    let Some(content) = c.root else { return };
    let n = |i: usize| &c.nodes[i];
    let group = |i: usize| matches!(n(i).ty, CType::Seq | CType::Or);
    out.push('(');
    let mut cur = content;
    loop {
        match n(cur).ty {
            CType::PcData => out.push_str("#PCDATA"),
            CType::Element => out.push_str(&n(cur).name),
            CType::Seq | CType::Or => {
                if cur != content
                    && let Some(par) = n(cur).parent
                    && (n(cur).ty != n(par).ty || n(cur).occur != Occur::Once)
                {
                    out.push('(');
                }
                match n(cur).c1 {
                    Some(c1) => {
                        cur = c1;
                        continue;
                    }
                    None => return,
                }
            }
        }
        while cur != content {
            let Some(par) = n(cur).parent else { return };
            if group(cur) && (n(cur).ty != n(par).ty || n(cur).occur != Occur::Once) {
                out.push(')');
            }
            out.push_str(occur_text(n(cur).occur));
            if Some(cur) == n(par).c1 {
                out.push_str(if n(par).ty == CType::Seq {
                    " , "
                } else {
                    " | "
                });
                match n(par).c2 {
                    Some(c2) => cur = c2,
                    None => return,
                }
                break;
            }
            cur = par;
        }
        if cur == content {
            break;
        }
    }
    out.push(')');
    out.push_str(occur_text(n(content).occur));
}

// ---- Reading a document -------------------------------------------------------

fn parse(input: &str) -> Res<Doc> {
    let mut b = Builder {
        doc: Doc::default(),
        entities: Default::default(),
        elements: Default::default(),
        tokenized: Default::default(),
        attlists: Default::default(),
        external_subset: false,
    };
    let mut p = Parser {
        s: input,
        i: 0,
        base: None,
    };
    if p.at("<?xml")
        && p.s
            .as_bytes()
            .get(5)
            .is_some_and(|c| is_blank(*c) || *c == b'?')
    {
        p.i += 5;
        p.need_blanks("in the XML declaration")?;
        p.expect("version")?;
        p.skip_blanks();
        p.expect("=")?;
        p.skip_blanks();
        let v = p.quoted()?;
        if !v.starts_with("1.") || v.len() < 3 || !v[2..].bytes().all(|b| b.is_ascii_digit()) {
            return p.err(format!("version {v} is not XML 1.x"));
        }
        b.doc.version = Some(v);
        let spaced = p.skip_blanks();
        if p.eat("encoding") {
            if !spaced {
                return p.err("space required before encoding");
            }
            p.skip_blanks();
            p.expect("=")?;
            p.skip_blanks();
            let e = p.quoted()?;
            if !e.starts_with(|c: char| c.is_ascii_alphabetic())
                || !e
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
            {
                return p.err(format!("{e} is not an encoding name"));
            }
            b.doc.encoding = Some(e);
        }
        let spaced = p.skip_blanks() || spaced;
        if p.eat("standalone") {
            if !spaced {
                return p.err("space required before standalone");
            }
            p.skip_blanks();
            p.expect("=")?;
            p.skip_blanks();
            b.doc.standalone = Some(match p.quoted()?.as_str() {
                "yes" => true,
                "no" => false,
                _ => return p.err("standalone is yes or no"),
            });
        }
        p.skip_blanks();
        p.expect("?>")?;
    }
    let mut root_seen = false;
    loop {
        p.skip_blanks();
        if p.i >= p.s.len() {
            break;
        }
        if p.at("<!--") {
            let c = comment(&mut p)?;
            b.doc.top.push(Node::Comment(c));
        } else if p.at("<?") {
            let (target, content) = pi(&mut p)?;
            b.doc.top.push(Node::Pi { target, content });
        } else if p.at("<!DOCTYPE") {
            if root_seen || b.doc.doctype.is_some() {
                return p.err("a DOCTYPE is not allowed here");
            }
            p.i += 9;
            p.need_blanks("after <!DOCTYPE")?;
            let mut dt = Doctype {
                name: p.name()?,
                ..Doctype::default()
            };
            let spaced = p.skip_blanks();
            if spaced {
                let (public, system) = b.external_id(&mut p, false)?;
                dt.public = public;
                dt.system = system;
                b.external_subset = dt.system.is_some();
            }
            p.skip_blanks();
            if p.eat("[") {
                b.internal_subset(&mut p, &mut dt)?;
                p.skip_blanks();
            }
            p.expect(">")?;
            b.doc.doctype_at = b.doc.top.len();
            b.doc.doctype = Some(dt);
        } else if p.at("<") && !root_seen && p.s[p.i + 1..].starts_with(is_name_start) {
            let mut stack = Vec::new();
            b.content(&mut p, &mut stack, 0, false)?;
            let root = stack.pop().map(|o| o.el);
            if let Some(el) = root {
                b.doc.top.push(Node::Element(el));
            }
            root_seen = true;
        } else if root_seen && p.at("<") && p.s[p.i + 1..].starts_with(is_name_start) {
            return p.err("Extra content at the end of the document");
        } else {
            return p.err(if root_seen {
                "Extra content at the end of the document"
            } else {
                "Start tag expected, '<' not found"
            });
        }
    }
    if !root_seen {
        return p.err("Start tag expected, '<' not found");
    }
    Ok(b.doc)
}

// ---- Writing it back ----------------------------------------------------------

struct Writer {
    out: String,
    /// The document names its encoding: a carriage return is `&#13;`, as
    /// libxml2 writes it then, not `&#xD;`.
    named: bool,
    /// It says UTF-8: characters past ASCII are written as they are, not
    /// as references.
    raw: bool,
    indent: String,
    /// How many levels the indentation grows to: libxml2's 60 characters.
    indent_max: usize,
}

impl Writer {
    fn indent(&mut self, level: usize) {
        for _ in 0..level.min(self.indent_max) {
            self.out.push_str(&self.indent.clone());
        }
    }

    fn text(&mut self, t: &str) {
        for c in t.chars() {
            match c {
                '<' => self.out.push_str("&lt;"),
                '>' => self.out.push_str("&gt;"),
                '&' => self.out.push_str("&amp;"),
                '\r' if self.named => self.out.push_str("&#13;"),
                '\r' => self.out.push_str("&#xD;"),
                c if !c.is_ascii() && !self.raw => {
                    self.out.push_str(&format!("&#x{:X};", u32::from(c)))
                }
                c => self.out.push(c),
            }
        }
    }

    fn attr(&mut self, a: &Attr) {
        self.out.push(' ');
        self.out.push_str(&a.name);
        self.out.push_str("=\"");
        for part in &a.value {
            match part {
                AttrPart::Ref(r) => {
                    self.out.push('&');
                    self.out.push_str(r);
                    self.out.push(';');
                }
                AttrPart::Text(t) => {
                    for c in t.chars() {
                        match c {
                            '\n' => self.out.push_str("&#10;"),
                            '\r' => self.out.push_str("&#13;"),
                            '\t' => self.out.push_str("&#9;"),
                            '"' => self.out.push_str("&quot;"),
                            '<' => self.out.push_str("&lt;"),
                            '>' => self.out.push_str("&gt;"),
                            '&' => self.out.push_str("&amp;"),
                            c if !c.is_ascii() && !self.raw => {
                                self.out.push_str(&format!("&#x{:X};", u32::from(c)))
                            }
                            c => self.out.push(c),
                        }
                    }
                }
            }
        }
        self.out.push('"');
    }

    fn node(&mut self, n: &Node, level: usize, format: bool) {
        match n {
            Node::Element(e) => self.element(e, level, format),
            Node::Text(t) => self.text(t),
            Node::CData(t) => {
                if t.is_empty() {
                    self.out.push_str("<![CDATA[]]>");
                } else {
                    self.out.push_str("<![CDATA[");
                    self.out.push_str(t);
                    self.out.push_str("]]>");
                }
            }
            Node::Comment(c) => {
                self.out.push_str("<!--");
                self.out.push_str(c);
                self.out.push_str("-->");
            }
            Node::Pi { target, content } => {
                self.out.push_str("<?");
                self.out.push_str(target);
                if let Some(c) = content {
                    self.out.push(' ');
                    self.out.push_str(c);
                }
                self.out.push_str("?>");
            }
            Node::EntityRef(r) => {
                self.out.push('&');
                self.out.push_str(r);
                self.out.push(';');
            }
        }
    }

    fn element(&mut self, e: &Element, level: usize, format: bool) {
        self.out.push('<');
        self.out.push_str(&e.name);
        // Namespace declarations first, as libxml2 keeps them apart.
        let is_ns = |a: &&Attr| a.name == "xmlns" || a.name.starts_with("xmlns:");
        for a in e.attrs.iter().filter(is_ns) {
            self.attr(a);
        }
        for a in e.attrs.iter().filter(|a| !is_ns(a)) {
            self.attr(a);
        }
        if e.children.is_empty() {
            self.out.push_str("/>");
            return;
        }
        let inner = format
            && !e
                .children
                .iter()
                .any(|c| matches!(c, Node::Text(_) | Node::CData(_) | Node::EntityRef(_)));
        self.out.push('>');
        if inner {
            self.out.push('\n');
        }
        for c in &e.children {
            if inner && matches!(c, Node::Element(_) | Node::Pi { .. } | Node::Comment(_)) {
                self.indent(level + 1);
            }
            self.node(c, level + 1, inner);
            if inner {
                self.out.push('\n');
            }
        }
        if inner {
            self.indent(level);
        }
        self.out.push_str("</");
        self.out.push_str(&e.name);
        self.out.push('>');
    }

    fn doctype(&mut self, d: &Doctype) {
        self.out.push_str("<!DOCTYPE ");
        self.out.push_str(&d.name);
        match (&d.public, &d.system) {
            (Some(p), s) => {
                self.out.push_str(" PUBLIC ");
                self.out.push_str(&quote(p));
                self.out.push(' ');
                self.out.push_str(&quote(s.as_deref().unwrap_or("")));
            }
            (None, Some(s)) => {
                self.out.push_str(" SYSTEM ");
                self.out.push_str(&quote(s));
            }
            (None, None) => {}
        }
        if !d.has_tables {
            self.out.push('>');
            return;
        }
        self.out.push_str(" [\n");
        for n in &d.notations {
            self.out.push_str(n);
            self.out.push('\n');
        }
        for decl in &d.decls {
            match decl {
                Decl::Element { name, ty } => {
                    self.out.push_str("<!ELEMENT ");
                    self.out.push_str(name);
                    match ty {
                        ElementType::Empty => self.out.push_str(" EMPTY"),
                        ElementType::Any => self.out.push_str(" ANY"),
                        ElementType::Mixed(c) | ElementType::Children(c) => {
                            self.out.push(' ');
                            dump_content(c, &mut self.out);
                        }
                    }
                    self.out.push_str(">\n");
                }
                Decl::Attlist {
                    elem,
                    name,
                    ty,
                    default,
                } => {
                    self.out
                        .push_str(&format!("<!ATTLIST {elem} {name} {ty} {default}>\n"));
                }
                Decl::Entity { text } => {
                    self.out.push_str(text);
                    self.out.push('\n');
                }
                Decl::Comment(c) => self.node(&Node::Comment(c.clone()), 0, false),
                Decl::Pi { target, content } => self.node(
                    &Node::Pi {
                        target: target.clone(),
                        content: content.clone(),
                    },
                    0,
                    false,
                ),
            }
        }
        self.out.push_str("]>");
    }
}

/// How a document is written: indented (`--format`) or not (`--noblanks`).
#[derive(Clone, Debug)]
pub struct Style {
    pub format: bool,
    /// One level's indentation: XMLLINT_INDENT, two spaces by default.
    pub indent: String,
}

/// `input` read and written back as xmllint does; an error says where
/// reading stopped.
pub fn rewrite(input: &[u8], style: &Style) -> Result<String, Error> {
    let input = input.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(input);
    if input.starts_with(b"\xFE\xFF") || input.starts_with(b"\xFF\xFE") {
        return Err(Error::input("tw xml reads UTF-8; this document is UTF-16"));
    }
    let text = std::str::from_utf8(input).map_err(|e| {
        Error::input(format!(
            "not UTF-8 at byte {}: tw xml reads UTF-8",
            e.valid_up_to()
        ))
        .at(Span::at(input, e.valid_up_to()))
    })?;
    // Line ends as XML reads them: \r\n and \r as \n.
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    let doc =
        parse(&text).map_err(|(at, msg)| Error::input(msg).at(Span::at(text.as_bytes(), at)))?;
    // Written as they are where the document says it is UTF-8 (or ASCII,
    // as libxml2 writes it); as references where it names another encoding,
    // which a document is read in only when it is all ASCII.
    let utf8 = doc.encoding.as_deref().is_some_and(|e| {
        ["utf-8", "utf8", "us-ascii", "ascii"]
            .iter()
            .any(|n| e.eq_ignore_ascii_case(n))
    });
    if let Some(e) = &doc.encoding
        && !utf8
        && !text.is_ascii()
    {
        return Err(Error::input(format!(
            "this document says it is {e}; tw xml reads UTF-8 (or a document that is all ASCII)"
        )));
    }
    let indent_max = if style.indent.is_empty() {
        0
    } else {
        60 / style.indent.len()
    };
    let mut w = Writer {
        out: String::new(),
        named: doc.encoding.is_some(),
        raw: utf8,
        indent: style.indent.clone(),
        indent_max,
    };
    let version = doc.version.as_deref().unwrap_or("1.0");
    w.out.push_str(&format!("<?xml version={}", quote(version)));
    if let Some(e) = &doc.encoding {
        w.out.push_str(&format!(" encoding={}", quote(e)));
    }
    match doc.standalone {
        Some(true) => w.out.push_str(" standalone=\"yes\""),
        Some(false) => w.out.push_str(" standalone=\"no\""),
        None => {}
    }
    w.out.push_str("?>\n");
    for (i, n) in doc.top.iter().enumerate() {
        if i == doc.doctype_at
            && let Some(d) = &doc.doctype
        {
            w.doctype(d);
            w.out.push('\n');
        }
        w.node(n, 0, style.format);
        w.out.push('\n');
    }
    Ok(w.out)
}

// ---- The tools ------------------------------------------------------------------

static REFERENCE: Option<Reference> = Some(Reference {
    name: "xmllint --format, --noblanks",
    version: "libxml2 2.9.14",
    compat: Compat::Output,
});

const fn meta(
    action: &'static str,
    id: &'static str,
    title: &'static str,
    summary: &'static str,
) -> Meta {
    Meta {
        id,
        group: "xml",
        action,
        action_flag: None,
        title,
        summary,
        category: Category::Data,
        keywords: &[
            "xml", "format", "pretty", "indent", "minify", "xmllint", "beautify",
        ],
        input: InputKind::Bytes,
        streams: false,
        reference: REFERENCE,
    }
}

static FORMAT_META: Meta = meta(
    "format",
    "xml.format",
    en!("XML format"),
    en!("XML checked and indented, as xmllint --format writes it"),
);
static MINIFY_META: Meta = meta(
    "minify",
    "xml.minify",
    en!("XML minify"),
    en!("XML checked, its ignorable blanks dropped, as xmllint --noblanks writes it"),
);

static FORMAT_PARAMS: &[Param] = &[
    Param {
        key: "indent",
        label: en!("Indent"),
        help: en!("Spaces a level; 2 as xmllint (its XMLLINT_INDENT)"),
        kind: ParamKind::Int { min: 0, max: 16 },
        default: Value::Int(2),
        cli: Cli {
            long: "indent",
            short: None,
        },
    },
    Param {
        key: "tab",
        label: en!("Tabs"),
        help: en!("A tab a level instead of spaces"),
        kind: ParamKind::Flag,
        default: Value::Bool(false),
        cli: Cli {
            long: "tab",
            short: None,
        },
    },
];

pub struct Format;
pub struct Minify;

impl Tool for Format {
    fn meta(&self) -> &'static Meta {
        &FORMAT_META
    }

    fn params(&self) -> &'static [Param] {
        FORMAT_PARAMS
    }

    fn run(&self, _ctx: &Ctx, input: &[u8], values: &Values) -> Result<Output, Error> {
        let indent = if values.flag(FORMAT_PARAMS, "tab") {
            "\t".to_owned()
        } else {
            " ".repeat(values.int(FORMAT_PARAMS, "indent").clamp(0, 16) as usize)
        };
        let input = input.to_vec();
        let style = Style {
            format: true,
            indent,
        };
        crate::on_large_stack(move || rewrite(&input, &style)).map(text)
    }

    fn detect(&self, sample: &[u8]) -> Option<Score> {
        let s = std::str::from_utf8(sample)
            .ok()?
            .trim_start_matches('\u{feff}')
            .trim_start();
        s.starts_with("<?xml").then_some(Score(70))
    }
}

impl Tool for Minify {
    fn meta(&self) -> &'static Meta {
        &MINIFY_META
    }

    fn run(&self, _ctx: &Ctx, input: &[u8], _values: &Values) -> Result<Output, Error> {
        let input = input.to_vec();
        let style = Style {
            format: false,
            indent: String::new(),
        };
        crate::on_large_stack(move || rewrite(&input, &style)).map(text)
    }
}

/// The document as a text result, which the CLI ends with its newline.
fn text(mut doc: String) -> Output {
    if doc.ends_with('\n') {
        doc.pop();
    }
    Output::Text(doc)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Differences from xmllint that are by design (the module's doc).
    const KNOWN: &[&str] = &["doctype-public"];

    /// Every document in testdata/xml as Debian 13's xmllint wrote it
    /// (scripts/xml_record.sh): its output, or for one it refused, an error.
    #[test]
    fn writes_as_xmllint_does() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/xml");
        let mut names: Vec<_> = std::fs::read_dir(&dir)
            .map(|d| {
                d.flatten()
                    .map(|e| e.path())
                    .filter(|p| p.extension().is_some_and(|x| x == "xml"))
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        assert!(names.len() > 70, "{}", dir.display());
        let mut failures = Vec::new();
        for path in names {
            let name = path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            let input = std::fs::read(&path).unwrap_or_default();
            for (mode, format) in [("format", true), ("noblanks", false)] {
                let want = std::fs::read_to_string(path.with_extension(mode)).unwrap_or_default();
                let (code, want) = want.split_once('\n').unwrap_or((&want, ""));
                let got = rewrite(
                    &input,
                    &Style {
                        format,
                        indent: "  ".into(),
                    },
                );
                let same = match (&got, code) {
                    (Ok(out), "0") => out == want,
                    (Err(_), c) => c != "0",
                    _ => false,
                };
                if same == KNOWN.contains(&name.as_str()) {
                    failures.push(format!(
                        "{name}.{mode}: {got:?}\n  xmllint ({code}): {want:?}"
                    ));
                }
            }
        }
        assert!(
            failures.is_empty(),
            "{} differ:\n{}",
            failures.len(),
            failures.join("\n")
        );
    }
}
