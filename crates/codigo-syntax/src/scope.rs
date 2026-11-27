// SPDX-License-Identifier: MIT OR Apache-2.0
//! The kinds of text a theme colours.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Scope {
    Keyword,
    /// `return`, `if`, `for`...
    Control,
    Operator,
    Punctuation,
    Bracket,
    String,
    Escape,
    Number,
    Constant,
    Boolean,
    Comment,
    DocComment,
    Function,
    Method,
    Macro,
    Type,
    BuiltinType,
    Variable,
    Parameter,
    Property,
    Attribute,
    Namespace,
    Label,
    Tag,
    Heading,
    Emphasis,
    Link,
    Literal,
}

impl Scope {
    /// Every scope, in the order a colour editor lists them.
    pub const ALL: [Scope; 28] = [
        Scope::Keyword,
        Scope::Control,
        Scope::Operator,
        Scope::Punctuation,
        Scope::Bracket,
        Scope::String,
        Scope::Escape,
        Scope::Number,
        Scope::Constant,
        Scope::Boolean,
        Scope::Comment,
        Scope::DocComment,
        Scope::Function,
        Scope::Method,
        Scope::Macro,
        Scope::Type,
        Scope::BuiltinType,
        Scope::Variable,
        Scope::Parameter,
        Scope::Property,
        Scope::Attribute,
        Scope::Namespace,
        Scope::Label,
        Scope::Tag,
        Scope::Heading,
        Scope::Emphasis,
        Scope::Link,
        Scope::Literal,
    ];

    /// A stable lower-case name, used in settings files.
    pub fn key(self) -> &'static str {
        match self {
            Scope::Keyword => "keyword",
            Scope::Control => "control",
            Scope::Operator => "operator",
            Scope::Punctuation => "punctuation",
            Scope::Bracket => "bracket",
            Scope::String => "string",
            Scope::Escape => "escape",
            Scope::Number => "number",
            Scope::Constant => "constant",
            Scope::Boolean => "boolean",
            Scope::Comment => "comment",
            Scope::DocComment => "doc_comment",
            Scope::Function => "function",
            Scope::Method => "method",
            Scope::Macro => "macro",
            Scope::Type => "type",
            Scope::BuiltinType => "builtin_type",
            Scope::Variable => "variable",
            Scope::Parameter => "parameter",
            Scope::Property => "property",
            Scope::Attribute => "attribute",
            Scope::Namespace => "namespace",
            Scope::Label => "label",
            Scope::Tag => "tag",
            Scope::Heading => "heading",
            Scope::Emphasis => "emphasis",
            Scope::Link => "link",
            Scope::Literal => "literal",
        }
    }

    /// The scope a [`Scope::key`] names.
    pub fn from_key(key: &str) -> Option<Scope> {
        Scope::ALL.into_iter().find(|s| s.key() == key)
    }

    /// The scope a tree-sitter capture name such as `function.method` stands for. The longest
    /// dotted prefix that is known wins; names that style nothing (`spell`, `none`, injection and
    /// local-scope captures) give `None`.
    pub fn from_capture(name: &str) -> Option<Scope> {
        let mut candidate = name;
        loop {
            if let Some(scope) = exact(candidate) {
                return scope;
            }
            candidate = &candidate[..candidate.rfind('.')?];
        }
    }
}

/// `Some(None)` is a capture that deliberately styles nothing.
fn exact(name: &str) -> Option<Option<Scope>> {
    use Scope::*;
    Some(Some(match name {
        "keyword"
        | "keyword.function"
        | "keyword.type"
        | "keyword.modifier"
        | "keyword.storage"
        | "keyword.import"
        | "keyword.operator"
        | "keyword.coroutine"
        | "keyword.directive"
        | "keyword.exception"
        | "keyword.debug"
        | "keyword.repeat.loop"
        | "storage"
        | "storage.type"
        | "include"
        | "preproc"
        | "define"
        | "type.qualifier"
        | "storageclass"
        | "self" => Keyword,
        "keyword.control"
        | "keyword.conditional"
        | "keyword.repeat"
        | "keyword.return"
        | "conditional"
        | "repeat"
        | "exception" => Control,
        "operator" => Operator,
        "punctuation" | "punctuation.delimiter" | "punctuation.special" | "delimiter" => {
            Punctuation
        }
        "punctuation.bracket" | "bracket" => Bracket,
        "string"
        | "string.special"
        | "string.documentation"
        | "string.regexp"
        | "string.special.key"
        | "string.special.path"
        | "string.special.url"
        | "character"
        | "character.special" => String,
        "string.escape" | "escape" => Escape,
        "number" | "number.float" | "float" => Number,
        "constant" | "constant.builtin" | "constant.character" | "constant.macro"
        | "variable.builtin" | "special" => Constant,
        "boolean" | "constant.builtin.boolean" => Boolean,
        "comment" | "comment.line" | "comment.block" => Comment,
        "comment.documentation" | "comment.doc" | "documentation" => DocComment,
        "function" | "function.call" | "function.builtin" | "function.special" | "constructor" => {
            Function
        }
        "function.method" | "function.method.call" | "method" | "method.call" => Method,
        "function.macro" | "macro" | "function.preproc" => Macro,
        "type" | "type.definition" | "type.parameter" | "class" | "interface" | "enum"
        | "struct" => Type,
        "type.builtin" => BuiltinType,
        "variable" | "variable.member" | "identifier" => Variable,
        "variable.parameter" | "parameter" | "variable.parameter.builtin" => Parameter,
        "property" | "field" | "attribute.key" | "key" => Property,
        "attribute" | "attribute.builtin" | "annotation" | "decorator" => Attribute,
        "module" | "namespace" | "module.builtin" => Namespace,
        "label" => Label,
        "tag" | "tag.builtin" | "tag.delimiter" | "tag.attribute" => Tag,
        "markup.heading" | "text.title" | "title" | "markup.heading.marker" => Heading,
        "markup.italic" | "markup.bold" | "markup.strong" | "text.emphasis" | "text.strong"
        | "emphasis" | "emphasis.strong" => Emphasis,
        "markup.link" | "markup.link.url" | "markup.link.label" | "text.uri" | "text.reference"
        | "link_uri" | "link_text" => Link,
        "markup.raw" | "markup.raw.block" | "markup.raw.inline" | "text.literal"
        | "markup.list" | "text.quote" | "markup.quote" => Literal,
        "spell" | "nospell" | "none" | "conceal" | "embedded" | "text" | "markup" | "local"
        | "injection" | "error" | "diff" | "namespace.builtin" => return Some(None),
        _ => return None,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_scope_has_a_unique_key_that_finds_it_again() {
        let mut seen = std::collections::HashSet::new();
        for s in Scope::ALL {
            assert!(seen.insert(s.key()), "{s:?} repeats a key");
            assert_eq!(Scope::from_key(s.key()), Some(s));
        }
        assert_eq!(Scope::from_key("nope"), None);
    }

    #[test]
    fn the_longest_known_prefix_wins() {
        assert_eq!(Scope::from_capture("keyword"), Some(Scope::Keyword));
        assert_eq!(Scope::from_capture("function.method"), Some(Scope::Method));
        assert_eq!(
            Scope::from_capture("function.call.lua"),
            Some(Scope::Function)
        );
        assert_eq!(
            Scope::from_capture("type.builtin"),
            Some(Scope::BuiltinType)
        );
        assert_eq!(
            Scope::from_capture("string.special.symbol"),
            Some(Scope::String)
        );
    }

    #[test]
    fn captures_that_style_nothing_give_none() {
        assert_eq!(Scope::from_capture("spell"), None);
        assert_eq!(Scope::from_capture("injection.language"), None);
        assert_eq!(Scope::from_capture("totally.unknown"), None);
    }
}
