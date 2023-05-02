// Copyright (C) 2018 Vincent Ambo <mail@tazj.in>
//
// nixdoc is free software: you can redistribute it and/or modify it
// under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <http://www.gnu.org/licenses/>.

//! This tool generates CommonMark from a Nix file defining library
//! functions, such as the files in `lib/` in the nixpkgs repository.
//!
//! TODO:
//! * extract function argument names
//! * extract line number & add it to generated output
//! * figure out how to specify examples (& leading whitespace?!)

extern crate structopt;
extern crate failure;
extern crate rnix;
extern crate rowan;

mod commonmark;

use self::commonmark::*;
use rnix::{
    SyntaxKind, SyntaxNode,
    ast::{AstToken, AttrpathValue, AttrSet, Comment, Expr, Lambda, Param}
};
use rowan::{WalkEvent, ast::AstNode};
use std::fs;

use std::path::PathBuf;
use structopt::StructOpt;

/// Command line arguments for nixdoc
#[derive(Debug, StructOpt)]
#[structopt(name = "nixdoc", about = "Generate CommonMark from Nix library functions")]
struct Options {
    /// Nix file to process.
    #[structopt(short = "f", long = "file", parse(from_os_str))]
    file: PathBuf,

    /// Name of the function category (e.g. 'strings', 'attrsets').
    #[structopt(short = "c", long = "category")]
    category: String,

    /// Description of the function category.
    #[structopt(short = "d", long = "description")]
    description: String,
}

#[derive(Debug)]
struct DocComment {
    /// Primary documentation string.
    doc: String,

    /// Optional type annotation for the thing being documented.
    doc_type: Option<String>,

    /// Usage example(s) (interpreted as a single code block)
    example: Option<String>,
}

#[derive(Debug)]
struct DocItem {
    name: String,
    comment: DocComment,
    args: Vec<Argument>,
}

/// Retrieve documentation comments.
fn retrieve_doc_comment(node: &SyntaxNode) -> Option<String> {
    // if the current node has a doc comment it'll be immediately preceded by that comment,
    // or there will be a whitespace token and *then* the comment tokens before it. we merge
    // multiple single-line comments into one large comment if they are on adjacent lines for
    // documentation simplicity.
    let mut token = node.first_token()?.prev_token()?;
    if token.kind() == SyntaxKind::TOKEN_WHITESPACE {
        token = token.prev_token()?;
    }
    if token.kind() != SyntaxKind::TOKEN_COMMENT {
        return None;
    }

    // backtrack to the start of the doc comment, allowing only a single multi-line comment
    // or adjacent single-line comments.
    // we don't care much about optimization here, doc comments aren't long enough for that.
    if token.text().starts_with("/*") {
        return Some(Comment::cast(token)?.text().to_string());
    }
    let mut result = String::new();
    while let Some(comment) = Comment::cast(token) {
        result.insert_str(0, comment.text());
        let ws = match comment.syntax().prev_token() {
            Some(t) if t.kind() == SyntaxKind::TOKEN_WHITESPACE => t,
            _ => break,
        };
        // only adjacent lines continue a doc comment, empty lines do not.
        match ws.text().strip_prefix("\n") {
            Some(trail) if !trail.contains("\n") => result.insert_str(0, ws.text()),
            _ => break,
        }
        token = match ws.prev_token() {
            Some(c) => c,
            _ => break,
        };
    }
    Some(result)
}

/// Transforms an AST node into a `DocItem` if it has a leading
/// documentation comment.
fn retrieve_doc_item(node: &AttrpathValue) -> Option<DocItem> {
    let comment = retrieve_doc_comment(node.syntax())?;
    let ident = node.attrpath().unwrap();
    // TODO this should join attrs() with '.' to handle whitespace, dynamic attrs and string
    // attrs. none of these happen in nixpkgs lib, and the latter two should probably be
    // rejected entirely.
    let item_name = ident.to_string();

    Some(DocItem {
        name: item_name,
        comment: parse_doc_comment(&comment),
        args: vec![],
    })
}

/// *Really* dumb, mutable, hacky doc comment "parser".
fn parse_doc_comment(raw: &str) -> DocComment {
    enum ParseState { Doc, Type, Example }

    let mut doc = String::new();
    let mut doc_type = String::new();
    let mut example = String::new();
    let mut state = ParseState::Doc;

    for line in raw.trim().lines() {
        let mut line = line.trim();

        if line.starts_with("Type:") {
            state = ParseState::Type;
            line = &line[5..]; // trim 'Type:'
        }

        if line.starts_with("Example:") {
            state = ParseState::Example;
            line = &line[8..]; // trim 'Example:'
        }

        match state {
            ParseState::Type => doc_type.push_str(line.trim()),
            ParseState::Doc => {
                doc.push_str(line.trim());
                doc.push('\n');
            },
            ParseState::Example => {
                example.push_str(line.trim());
                example.push('\n');
            },
        }
    }

    let f = |s: String| if s.is_empty() { None } else { Some(s) };

    DocComment {
        doc: doc.trim().into(),
        doc_type: f(doc_type),
        example: f(example),
    }
}

/// Traverse a Nix lambda and collect the identifiers of arguments
/// until an unexpected AST node is encountered.
fn collect_lambda_args(mut lambda: Lambda) -> Vec<Argument> {
    let mut args = vec![];

    loop {
        match lambda.param().unwrap() {
            Param::IdentParam(id) => {
                args.push(Argument::Flat(SingleArg {
                    name: id.to_string(),
                    doc: retrieve_doc_comment(id.syntax()),
                }));
            },
            Param::Pattern(pat) => {
                let pattern_vec: Vec<_> = pat
                    .pat_entries()
                    .map(|entry| SingleArg {
                        name: entry.ident().unwrap().to_string(),
                        doc: retrieve_doc_comment(entry.syntax()),
                    })
                    .collect();

                args.push(Argument::Pattern(pattern_vec));
            },
        }

        // Curried or not?
        match lambda.body() {
            Some(Expr::Lambda(inner)) => lambda = inner,
            _ => break,
        }
    }

    args
}

/// Traverse the arena from a top-level SetEntry and collect, where
/// possible:
///
/// 1. The identifier of the set entry itself.
/// 2. The attached doc comment on the entry.
/// 3. The argument names of any curried functions (pattern functions
///    not yet supported).
fn collect_entry_information(entry: AttrpathValue) -> Option<DocItem> {
    let doc_item = retrieve_doc_item(&entry)?;

    if let Some(Expr::Lambda(l)) = entry.value() {
        Some(DocItem { args: collect_lambda_args(l), ..doc_item })
    } else {
        Some(doc_item)
    }
}

fn main() {
    let opts = Options::from_args();
    let src = fs::read_to_string(&opts.file).unwrap();
    let nix = rnix::Root::parse(&src).ok().expect("failed to parse input");

    let entries: Vec<_> = nix.syntax().preorder()
        .filter_map(|ev| match ev {
            WalkEvent::Enter(n) => Some(n),
            _ => None,
        })
        .filter_map(AttrSet::cast)
        .flat_map(|n| n.syntax().children())
        .filter_map(AttrpathValue::cast)
        .filter_map(collect_entry_information)
        .map(|d| ManualEntry {
            category: opts.category.clone(),
            name: d.name,
            description: d.comment.doc
                .split("\n\n")
                .map(|s| s.to_string())
                .collect(),
            fn_type: d.comment.doc_type,
            example: d.comment.example,
            args: d.args,
        })
        .collect();

    println!("# {} {{#sec-functions-library-{}}}\n", &opts.description, opts.category);

    for entry in entries {
        entry.write_section().expect("Failed to write section")
    }
}
