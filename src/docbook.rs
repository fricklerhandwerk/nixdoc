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

//! This module implements DocBook XML output for a struct
//! representing a single entry in th emanual.

use std::io::Write;
use xml::writer::{EventWriter, XmlEvent};
use failure::Error;

/// Write a plain start element (most commonly used).
pub fn element<W: Write>(w: &mut EventWriter<W>, name: &str) -> Result<(), Error> {
    w.write(XmlEvent::start_element(name))?;
    Ok(())
}

/// End an element.
pub fn end<W: Write>(w: &mut EventWriter<W>) -> Result<(), Error> {
    w.write(XmlEvent::end_element())?;
    Ok(())
}

/// Write a string.
pub fn string<W: Write>(w: &mut EventWriter<W>, content: &str) -> Result<(), Error> {
    w.write(XmlEvent::characters(content))?;
    Ok(())
}

/// Represent a single function argument name and its (optional)
/// doc-string.
#[derive(Debug)]
pub struct SingleArg {
    pub name: String,
    pub doc: Option<String>,
}

/// Represent a function argument, which is either a flat identifier
/// or a pattern set.
#[derive(Debug)]
pub enum Argument {
    /// Flat function argument (e.g. `n: n * 2`).
    Flat(SingleArg),

    /// Pattern function argument (e.g. `{ name, age }: ...`)
    Pattern(Vec<SingleArg>),
}

impl Argument {
    /// Write DocBook structure for a single function argument.
    fn write_argument_xml<W: Write>(self, w: &mut EventWriter<W>) -> Result<(), Error> {
        match self {
            // Write a flat argument entry.
            Argument::Flat(arg) => {
                element(w, "varlistentry")?;

                element(w, "term")?;
                element(w, "varname")?;
                string(w, &arg.name)?;
                end(w)?;
                end(w)?;

                element(w, "listitem")?;
                element(w, "para")?;
                string(w, arg.doc.unwrap_or("Function argument".into()).trim())?;
                end(w)?;
                end(w)?;

                end(w)?;
            },

            // Write a pattern argument entry and its individual
            // parameters as a nested structure.
            Argument::Pattern(pattern_args) => {
                element(w, "varlistentry")?;

                element(w, "term")?;
                element(w, "varname")?;
                string(w, "pattern")?;
                end(w)?;
                end(w)?;

                element(w, "listitem")?;
                element(w, "para")?;
                string(w, "Structured function argument")?;
                end(w)?;

                element(w, "variablelist")?;
                for pattern_arg in pattern_args {
                    Argument::Flat(pattern_arg)
                        .write_argument_xml(w)?;
                }
                end(w)?;
                end(w)?;
                end(w)?;
            },
        }

        Ok(())
    }
}

/// Represents a single manual section describing a library function.
#[derive(Debug)]
pub struct ManualEntry {
    /// Name of the function category (e.g. 'strings', 'trivial', 'attrsets')
    pub category: String,

    /// Name of the section (used as the title)
    pub name: String,

    /// Type signature (if provided). This is not actually a checked
    /// type signature in any way.
    pub fn_type: Option<String>,

    /// Primary description of the entry. Each entry is written as a
    /// separate paragraph.
    pub description: Vec<String>,

    /// Usage example for the entry.
    pub example: Option<String>,

    /// Arguments of the function
    pub args: Vec<Argument>,
}

impl ManualEntry {
    /// Write a single DocBook entry for a documented Nix function.
    pub fn write_section_xml<W: Write>(self, w: &mut EventWriter<W>) -> Result<(), Error> {
        let title = format!("lib.{}.{}", self.category, self.name);
        let ident = format!("lib.{}.{}", self.category, self.name.replace("'", "-prime"));

        // <section ...
        w.write(XmlEvent::start_element("section")
                .attr("xml:id", format!("function-library-{}", ident).as_str()))?;

        // <title> ...
        element(w, "title")?;
        element(w, "function")?;
        string(w, title.as_str())?;
        end(w)?;
        end(w)?;

        // Write an include header that will load manually written
        // documentation for this function if required.
        let override_path = format!("./overrides/{}.xml", ident);
        w.write(XmlEvent::start_element("xi:include")
                .attr("href", &override_path))?;
        element(w, "xi:fallback")?;

        // <subtitle> (type signature)
        if let Some(t) = &self.fn_type {
            element(w, "subtitle")?;
            element(w, "literal")?;
            string(w, t)?;
            end(w)?;
            end(w)?;
        }

        // Primary doc string
        // TODO: Split paragraphs?
        for paragraph in &self.description {
            element(w, "para")?;
            string(w, paragraph)?;
            end(w)?;
        }

        // Function argument names
        if !self.args.is_empty() {
            element(w, "variablelist")?;

            for arg in self.args {
                arg.write_argument_xml(w)?;
            }

            end(w)?;
        }

        // Example program listing (if applicable)
        //
        // TODO: In grhmc's version there are multiple (named)
        // examples, how can this be achieved automatically?
        if let Some(example) = &self.example {
            element(w, "example")?;

            element(w, "title")?;

            element(w, "function")?;
            string(w, title.as_str())?;
            end(w)?;

            string(w, " usage example")?;
            end(w)?;

            element(w, "programlisting")?;
            w.write(XmlEvent::cdata(example))?;
            end(w)?;

            end(w)?;
        }

        // </xi:fallback></xi:include>
        end(w)?;
        end(w)?;

        // Include link to function location (location information is
        // generated by a separate script in nixpkgs)
        w.write(XmlEvent::start_element("xi:include")
                .attr("href", "./locations.xml")
                .attr("xpointer", &ident))?;
        end(w)?;

        // </section>
        end(w)?;

        Ok(())
    }
}
