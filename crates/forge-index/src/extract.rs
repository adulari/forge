//! Tree-sitter extraction: a Rust source file → a flat list of symbol [`RawNode`]s plus the
//! `contains` parent→child relationships between them. Pure (no I/O, no store): the caller owns
//! reading the file and persisting the result. Language support starts with Rust; adding a
//! grammar is additive here.

use tree_sitter::{Node as TsNode, Parser};

/// A symbol pulled from the AST, before it gets a persisted `SymbolId`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawNode {
    pub kind: NodeKind,
    pub name: String,
    /// Enclosing named items joined by `::` (e.g. `Session::run_turn`), the symbol included.
    pub qualname: String,
    pub signature: Option<String>,
    pub span_start: usize,
    pub span_end: usize,
    pub line_start: u32,
    /// Index into the returned `Vec<RawNode>` of the enclosing symbol (for a `contains` edge);
    /// `None` for a top-level item.
    pub parent: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    Function,
    Method,
    Struct,
    Enum,
    Trait,
    Impl,
    Const,
    Module,
    TypeAlias,
}

impl NodeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            NodeKind::Function => "function",
            NodeKind::Method => "method",
            NodeKind::Struct => "struct",
            NodeKind::Enum => "enum",
            NodeKind::Trait => "trait",
            NodeKind::Impl => "impl",
            NodeKind::Const => "const",
            NodeKind::Module => "module",
            NodeKind::TypeAlias => "type",
        }
    }
}

/// Parse Rust source and return its symbols (depth-first, parents before children). Returns an
/// empty vec if the grammar fails to load or the source doesn't parse into a tree.
pub fn extract_rust(src: &str) -> Vec<RawNode> {
    let mut parser = Parser::new();
    if parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .is_err()
    {
        return Vec::new();
    }
    let Some(tree) = parser.parse(src, None) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    walk(tree.root_node(), src, None, &[], false, &mut out);
    out
}

fn walk(
    node: TsNode,
    src: &str,
    parent: Option<usize>,
    scope: &[String],
    in_impl: bool,
    out: &mut Vec<RawNode>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        let (this_parent, this_scope, child_in_impl) =
            if let Some((kind, name)) = classify(&child, src, in_impl) {
                let qualname = {
                    let mut q = scope.to_vec();
                    q.push(name.clone());
                    q.join("::")
                };
                out.push(RawNode {
                    kind,
                    name: name.clone(),
                    qualname: qualname.clone(),
                    signature: signature(&child, src),
                    span_start: child.start_byte(),
                    span_end: child.end_byte(),
                    line_start: child.start_position().row as u32 + 1,
                    parent,
                });
                let idx = out.len() - 1;
                let mut next_scope = scope.to_vec();
                next_scope.push(name);
                (Some(idx), next_scope, in_impl || kind == NodeKind::Impl)
            } else {
                (parent, scope.to_vec(), in_impl)
            };
        walk(child, src, this_parent, &this_scope, child_in_impl, out);
    }
}

/// Map a tree-sitter node to a [`NodeKind`] + its declared name, or `None` if it isn't a symbol
/// we index. A `function_item` inside an `impl` block is a method.
fn classify(node: &TsNode, src: &str, in_impl: bool) -> Option<(NodeKind, String)> {
    let kind = match node.kind() {
        "function_item" | "function_signature_item" => {
            if in_impl {
                NodeKind::Method
            } else {
                NodeKind::Function
            }
        }
        "struct_item" => NodeKind::Struct,
        "enum_item" => NodeKind::Enum,
        "trait_item" => NodeKind::Trait,
        "const_item" | "static_item" => NodeKind::Const,
        "mod_item" => NodeKind::Module,
        "type_item" => NodeKind::TypeAlias,
        "impl_item" => {
            // The "name" of an impl is the type it's for (skip the trait for `impl X for Y`).
            let name = node
                .child_by_field_name("type")
                .and_then(|n| text(&n, src))
                .unwrap_or_else(|| "impl".to_string());
            return Some((NodeKind::Impl, name));
        }
        _ => return None,
    };
    let name = node
        .child_by_field_name("name")
        .and_then(|n| text(&n, src))?;
    Some((kind, name))
}

/// A one-line signature: the node's text up to the body `{` / `;` / `=`, collapsed to one line.
fn signature(node: &TsNode, src: &str) -> Option<String> {
    let full = text(node, src)?;
    let head: String = full
        .chars()
        .take_while(|&c| c != '{' && c != ';' && c != '\n')
        .collect();
    let head = head.split_whitespace().collect::<Vec<_>>().join(" ");
    (!head.is_empty()).then_some(head)
}

fn text(node: &TsNode, src: &str) -> Option<String> {
    node.utf8_text(src.as_bytes()).ok().map(|s| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRC: &str = r#"
pub mod net {
    pub struct Session { id: String }

    impl Session {
        pub fn run_turn(&self, prompt: &str) -> String { String::new() }
    }

    pub trait Router { fn route(&self); }
    pub enum Tier { Trivial, Complex }
    pub const MAX: usize = 8;
    pub fn helper() {}
    type Alias = u32;
}
"#;

    fn find<'a>(nodes: &'a [RawNode], name: &str) -> &'a RawNode {
        nodes.iter().find(|n| n.name == name).expect("node present")
    }

    #[test]
    fn extracts_each_symbol_kind() {
        let nodes = extract_rust(SRC);
        assert_eq!(find(&nodes, "net").kind, NodeKind::Module);
        assert_eq!(find(&nodes, "Session").kind, NodeKind::Struct);
        assert_eq!(find(&nodes, "Router").kind, NodeKind::Trait);
        assert_eq!(find(&nodes, "Tier").kind, NodeKind::Enum);
        assert_eq!(find(&nodes, "MAX").kind, NodeKind::Const);
        assert_eq!(find(&nodes, "helper").kind, NodeKind::Function);
        assert_eq!(find(&nodes, "Alias").kind, NodeKind::TypeAlias);
    }

    #[test]
    fn function_in_impl_is_a_method_with_qualname() {
        let nodes = extract_rust(SRC);
        let m = find(&nodes, "run_turn");
        assert_eq!(m.kind, NodeKind::Method);
        assert_eq!(m.qualname, "net::Session::run_turn");
        assert!(m.signature.as_deref().unwrap().contains("run_turn"));
        // Its parent is the impl block.
        let parent = m.parent.map(|i| nodes[i].kind);
        assert_eq!(parent, Some(NodeKind::Impl));
    }

    #[test]
    fn empty_or_unparseable_source_yields_no_nodes() {
        assert!(extract_rust("").is_empty());
    }
}
