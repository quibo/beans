use super::ast::Ast;
use crate::build_system;
use crate::builder::Buildable;
use crate::error::{Error, ErrorKind, Result, WarningSet};
use crate::lexer::TerminalId;
use crate::parser::{Parser, AST};
use crate::regex::{CompiledRegex, RegexBuilder};
use crate::stream::StringStream;
use crate::typed::Tree;
use bincode::deserialize;
use newty::newty;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::rc::Rc;

newty! {
    pub id TokenId
}

newty! {
    pub set Ignores [TerminalId]
}

newty! {
    #[derive(Serialize, Deserialize)]
    pub map Errors(Rc<str>) [TerminalId]
}

newty! {
    #[derive(Serialize, Deserialize)]
    pub map Descriptions(Rc<str>) [TerminalId]
}

/// # Summary
///
/// `LexerGrammar` is a grammar for a lexer. It is already setup.
/// Should be built with a `LexerGrammarBuilder`.
#[derive(Debug, Serialize, Deserialize)]
pub struct LexerGrammar {
    pattern: CompiledRegex,
    names: Vec<String>,
    ignores: Ignores,
    errors: Errors,
    descriptions: Descriptions,
    default_allowed: Vec<TerminalId>,
    name_map: HashMap<String, TerminalId>,
}

impl LexerGrammar {
    pub fn new(
        pattern: CompiledRegex,
        names: Vec<String>,
        ignores: Ignores,
        errors: Errors,
        descriptions: Descriptions,
    ) -> Self {
        let mut name_map = HashMap::new();
        for (i, name) in names.iter().enumerate() {
            let id = TerminalId(i);
            name_map.insert(name.clone(), id);
        }
        let default_allowed = ignores.0.ones().map(TerminalId).collect();
        Self {
            pattern,
            names,
            ignores,
            errors,
            descriptions,
            default_allowed,
            name_map,
        }
    }

    pub fn default_allowed(&self) -> impl Iterator<Item = TerminalId> + '_ {
        self.default_allowed.iter().copied()
    }

    pub fn name(&self, idx: TerminalId) -> &str {
        &self.names[idx.0][..]
    }

    pub fn contains(&self, name: &str) -> bool {
        self.name_map.contains_key(name)
    }

    pub fn ignored(&self, idx: TerminalId) -> bool {
        self.ignores.contains(idx)
    }

    pub fn err_message(&self, idx: TerminalId) -> Option<&str> {
        self.errors.get(&idx).map(|x| &**x)
    }

    pub fn description_of(&self, idx: TerminalId) -> Option<&str> {
        self.descriptions.get(&idx).map(|x| &**x)
    }

    pub fn pattern(&self) -> &CompiledRegex {
        &self.pattern
    }

    pub fn has_token(&self, token: &str) -> bool {
        self.name_map.contains_key(token)
    }

    pub fn id(&self, name: &str) -> Option<TerminalId> {
        self.name_map.get(name).copied()
    }
}

impl Buildable for LexerGrammar {
    const RAW_EXTENSION: &'static str = "lx";
    const COMPILED_EXTENSION: &'static str = "clx";
    const AST_EXTENSION: &'static str = "lx.ast";

    fn build_from_ast(ast: AST) -> Result<Self> {
        let typed_ast = Ast::read(ast);

        let warnings = WarningSet::empty();
        let mut ignores = Ignores::with_raw_capacity(typed_ast.terminals.len());
        let mut errors = Errors::new();
        let mut descriptions = Descriptions::new();
        let mut names = Vec::new();
        let mut regex_builder = RegexBuilder::new();
        let mut found_identifiers = HashMap::new();

        for terminal in typed_ast.terminals {
            let id = TerminalId(names.len());
            if terminal.ignore || terminal.unwanted {
                ignores.put(id);
            }
            if terminal.unwanted {
                if let Some(ref message) = terminal.comment {
                    errors.insert(id, message.clone());
                } else {
                    return ErrorKind::LexerGrammarUnwantedNoDescription {
                        token: terminal.name.to_string(),
                        span: todo!(),
                    }
                    .err();
                }
            }
            if let Some(comment) = terminal.comment {
                descriptions.insert(id, comment);
            }
            names.push(terminal.name.to_string());

            if let Some(_span) = found_identifiers.insert(terminal.name.clone(), ()) {
                return ErrorKind::GrammarDuplicateDefinition {
                    message: terminal.name.to_string(),
                    span: todo!(),
                    old_span: todo!(),
                }
                .err();
            }

            regex_builder = regex_builder
                .with_named_regex(&terminal.regex, terminal.name.to_string(), terminal.keyword)
                .map_err(|error| {
                    Error::new(ErrorKind::RegexError {
                        message: error.message,
                        span: todo!(),
                    })
                })?;
        }
        let re = regex_builder.build();
        warnings.with_ok(Self::new(re, names, ignores, errors, descriptions))
    }

    fn build_from_compiled(blob: &[u8]) -> Result<Self> {
        WarningSet::empty().with_ok(deserialize(blob)?)
    }

    fn build_from_plain(mut source: StringStream) -> Result<Self> {
        let mut warnings = WarningSet::empty();
        let (lexer, parser) = build_system!(
            lexer => "lexer.clx",
            parser => "lexer.cgr",
        )?
        .unpack_into(&mut warnings);
        let mut input = lexer.lex(&mut source);
        let result = parser.parse(&mut input)?.unpack_into(&mut warnings);
        let grammar = Self::build_from_ast(result.tree)?.unpack_into(&mut warnings);
        warnings.with_ok(grammar)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn grammar_parser_regex() {
        assert_eq!(
            *LexerGrammar::build_from_plain(StringStream::new(
                Path::new("whatever"),
                "A ::= wot!"
            ))
            .unwrap()
            .unwrap()
            .pattern(),
            RegexBuilder::new()
                .with_named_regex("wot!", String::from("A"), false)
                .unwrap()
                .build(),
        );
        assert_eq!(
            *LexerGrammar::build_from_plain(StringStream::new(
                Path::new("whatever"),
                "B ::= wot!  "
            ))
            .unwrap()
            .unwrap()
            .pattern(),
            RegexBuilder::new()
                .with_named_regex("wot!  ", String::from("B"), false)
                .unwrap()
                .build()
        );
        assert_eq!(
            *LexerGrammar::build_from_plain(StringStream::new(
                Path::new("whatever"),
                "A ::= wot!\n\nB ::= wheel"
            ))
            .unwrap()
            .unwrap()
            .pattern(),
            RegexBuilder::new()
                .with_named_regex("wot!", String::from("A"), false)
                .unwrap()
                .with_named_regex("wheel", String::from("B"), false)
                .unwrap()
                .build()
        );
        assert_eq!(
            *LexerGrammar::build_from_plain(StringStream::new(Path::new("whatever"), ""))
                .unwrap()
                .unwrap()
                .pattern(),
            RegexBuilder::new().build()
        );
    }
    #[test]
    fn lexer_grammar() {
        let grammar = LexerGrammar::build_from_plain(StringStream::new(
            Path::new("whatever"),
            "ignore A ::= [ ]\nignore B ::= bbb\nC ::= ccc",
        ))
        .unwrap()
        .unwrap();
        assert_eq!(grammar.name(TerminalId(0)), "A");
        assert!(grammar.ignored(0.into()));
        assert_eq!(grammar.name(TerminalId(1)), "B");
        assert!(grammar.ignored(1.into()));
        assert_eq!(grammar.name(TerminalId(2)), "C");
        assert!(!grammar.ignored(2.into()));
    }

    #[test]
    fn grammar_report() {
        let grammar = LexerGrammar::build_from_plain(StringStream::new(
            Path::new("<grammar report>"),
            r#"ignore COMMENT ::= /\*([^*]|\*[^/])\*/
(unclosed comment) unwanted ECOMMENT ::= /\*([^*]|\*[^/])"#,
        ))
        .unwrap()
        .unwrap();
        assert_eq!(1, grammar.errors.len());
        assert_eq!(
            "unclosed comment",
            &**grammar.errors.get(&TerminalId(1)).unwrap()
        );
    }
}
