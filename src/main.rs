use anyhow::Context;
use beans::error::{Error, WarningSet, WithWarnings};
use beans::lexer::{LexerBuilder, LexerGrammar, LexerGrammarBuilder};
use beans::parser::earley::{EarleyGrammarBuilder, print_sets, print_final_sets};
use beans::parser::grammarparser::GrammarBuilder;
use beans::parser::parser::{Parser, Value, AST};
use beans::parser::EarleyParser;
use beans::regex::Allowed;
use beans::stream::StringStream;
use bincode::{deserialize, serialize};
use clap::{Parser as CliParser, Subcommand};
use ptree::{print_tree, TreeBuilder};
use std::fs::File;
use std::io::{prelude::*, stdout, BufWriter};
use std::path::PathBuf;

#[derive(CliParser)]
#[command(author, version, about, long_about=None)]
#[command(propagate_version = true)]
struct Cli {
    #[command(subcommand)]
    action: Action,
}

#[derive(Subcommand)]
enum Action {
    #[command(subcommand)]
    Compile(CompileAction),
    Lex {
        #[arg(short = 'l', long = "lexer")]
        lexer_grammar: PathBuf,
        source: PathBuf,
    },
    Parse {
	/// Show the intermediate table used by the Earley parser
        #[arg(short, long)]
        table: bool,
	/// Show the final table used by the Earley parser
        #[arg(short, long)]
        final_table: bool,
	/// Specify the lexer's grammar
        #[arg(short, long = "lexer")]
        lexer_grammar: PathBuf,
	/// Specify the parser's grammar
        #[arg(short, long = "parser")]
        parser_grammar: PathBuf,
	/// The file to parse
        source: PathBuf,
    },
}

#[derive(Subcommand)]
enum CompileAction {
    /// Compile a lexer grammar
    Lexer {
        lexer_grammar: PathBuf,
        #[arg(short = 'o', long = "output")]
        output_path: Option<PathBuf>,
    },
    /// Compile a parser grammar
    Parser {
        /// The path to the grammar
        parser_grammar: PathBuf,
        /// The path of the resulting compiled blob
        #[arg(short = 'o', long = "output")]
        output_path: Option<PathBuf>,
        /// The path to the lexer grammar on top of which the parser relies
        #[arg(short = 'l', long = "lexer")]
        lexer_path: PathBuf,
    },
}

fn compile(compile_action: CompileAction) -> anyhow::Result<WithWarnings<()>> {
    let mut warnings = WarningSet::default();
    match compile_action {
        CompileAction::Lexer {
            lexer_grammar: mut lexer_grammar_path,
            output_path,
        } => {
            let lexer_grammar =
                LexerGrammarBuilder::from_file(lexer_grammar_path.as_path())?
                    .unpack_into(&mut warnings)
                    .build()?
                    .unpack_into(&mut warnings);
            let res = serialize(&lexer_grammar)?;
            let output = match output_path {
                Some(output) => output,
                None => {
                    if !lexer_grammar_path.set_extension("clx") {
                        return Err(Error::SameOutputAndInput.into());
                    }
                    lexer_grammar_path
                }
            };
            let mut output_fs = File::create(output.as_path())?;
            output_fs.write_all(&res).context(format!(
                "Could not write to file {}",
                output.display()
            ))?;
        }
        CompileAction::Parser {
            parser_grammar: mut parser_grammar_path,
            output_path,
            lexer_path,
        } => {
            let lexer_grammar = if let Some("clx") =
                lexer_path.extension().and_then(|x| x.to_str())
            {
                let mut fd = File::open(lexer_path.as_path())?;
                let mut buffer = Vec::new();
                fd.read_to_end(&mut buffer)?;
                bincode::deserialize(&buffer)?
            } else {
                LexerGrammarBuilder::from_file(lexer_path.as_path())?
                    .unpack_into(&mut warnings)
                    .build()?
                    .unpack_into(&mut warnings)
            };
            let lexer = LexerBuilder::from_grammar(lexer_grammar).build();
            let parser_grammar = EarleyGrammarBuilder::default()
                .with_file(parser_grammar_path.as_path())?
                .unpack_into(&mut warnings)
                .build(&lexer)?
                .unpack_into(&mut warnings);
            let output = match output_path {
                Some(output) => output,
                None => {
                    if !parser_grammar_path.set_extension("cgr") {
                        return Err(Error::SameOutputAndInput.into());
                    }
                    parser_grammar_path
                }
            };
            let mut output_fd = File::create(output)?;
            output_fd.write_all(&serialize(&parser_grammar)?)?;
        }
    }
    warnings.with_ok(())
}

fn build_tree(tree: &mut TreeBuilder, ast: &AST) {
    match ast {
        AST::Node { attributes, .. } => {
            for (key, value) in attributes.iter() {
                tree.begin_child(key.to_string());
                build_tree(tree, value);
                tree.end_child();
            }
        }
        AST::Literal(Value::Int(i)) => {
            tree.add_empty_child(i.to_string());
        }
        AST::Literal(Value::Str(string)) => {
            tree.add_empty_child(string.to_string());
        }
        AST::Literal(Value::Float(f)) => {
            tree.add_empty_child(f.to_string());
        }
        AST::Literal(Value::Bool(b)) => {
            tree.add_empty_child(b.to_string());
        }
        AST::Terminal(ter) => {
            tree.add_empty_child(ter.name().to_string());
        }
    }
}

fn main() -> anyhow::Result<()> {
    let Cli { action } = Cli::parse();
    let mut warnings = WarningSet::default();
    match action {
        Action::Compile(compile_action) => {
            compile(compile_action)?.unpack_into(&mut warnings)
        }
        Action::Lex {
            lexer_grammar: lexer_grammar_path,
            source,
        } => {
            let lexer_grammar =
                LexerGrammar::deserialize_from(lexer_grammar_path)?
                    .unpack_into(&mut warnings);
            let lexer = LexerBuilder::from_grammar(lexer_grammar).build();
            let mut stream =
                StringStream::from_file(source)?.unpack_into(&mut warnings);
            let mut lexed_stream = lexer.lex(&mut stream);
            let mut output_buffer = BufWriter::new(stdout());
            while let Some(token) =
                lexed_stream.next(Allowed::All)?.unpack_into(&mut warnings)
            {
                write!(output_buffer, "{} {{ ", token.name())?;
                for (key, value) in token.attributes().iter() {
                    write!(output_buffer, "{}: {}, ", key, value)?;
                }
                writeln!(output_buffer, "}}")?;
            }
            output_buffer.flush()?;
        }
        Action::Parse {
            table: print_table,
            final_table: print_final_table,
            lexer_grammar: lexer_grammar_path,
            parser_grammar: parser_grammar_path,
            source,
        } => {
            let lexer_grammar =
                LexerGrammar::deserialize_from(lexer_grammar_path)?
                    .unpack_into(&mut warnings);
            let lexer = LexerBuilder::from_grammar(lexer_grammar).build();
            let parser_grammar = if let Some("cgr") =
                parser_grammar_path.extension().and_then(|x| x.to_str())
            {
                let mut buffer = Vec::new();
                let mut fd = File::open(parser_grammar_path.as_path())?;
                fd.read_to_end(&mut buffer)?;
                deserialize(&buffer)?
            } else {
                EarleyGrammarBuilder::new(parser_grammar_path.as_path())
                    .build(&lexer)?
                    .unpack_into(&mut warnings)
            };
            let parser = EarleyParser::new(parser_grammar);
            // let (table, raw_input) =
            //     EarleyParser::recognise(
            // 	    &parser,
            //         &mut lexer.lex(
            //             &mut StringStream::from_file(source)?
            //                 .unpack_into(&mut warnings),
            //         ),
            //     )?
            //     .unpack_into(&mut warnings);
            // println!("{:#?}\n{}", table, raw_input.len());
            let mut stream =
                StringStream::from_file(source)?.unpack_into(&mut warnings);
            let mut input = lexer.lex(&mut stream);
            let (table, raw_input) =
                parser.recognise(&mut input)?.unpack_into(&mut warnings);
	    if print_table {
		println!(" ### TABLE ###");
		print_sets(&table, &parser);
	    }
            let forest = parser
                .to_forest(&table, &raw_input)?
                .unpack_into(&mut warnings);
            if print_final_table {
		println!(" ### FINAL TABLE ###");
		print_final_sets(&forest, &parser);
	    }
            let ast = parser.select_ast(&forest, &raw_input);
            let mut tree = TreeBuilder::new(String::from("AST"));
            build_tree(&mut tree, &ast);
            let tree = tree.build();
            print_tree(&tree)?;
        }
    }
    for warning in warnings.iter() {
        println!("{}", warning);
    }
    Ok(())
}