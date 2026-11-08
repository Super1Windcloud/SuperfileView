//! Document search options and replacement semantics, independent of editor internals.
use regex::{Regex, RegexBuilder};
use std::{ops::Range, rc::Rc};

#[derive(Default)]
pub struct SearchMatcher {
    text: String,
    expression: Option<Regex>,
    ranges: Rc<Vec<Range<usize>>>,
    current: usize,
    regular_expression: bool,
}
impl SearchMatcher {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn update(&mut self, text: &str) {
        self.text = text.to_owned();
        self.collect();
    }
    pub fn update_options(
        &mut self,
        query: &str,
        case_sensitive: bool,
        whole_word: bool,
        regular_expression: bool,
    ) -> Result<(), regex::Error> {
        self.regular_expression = regular_expression;
        self.expression = None;
        self.current = 0;
        self.ranges = Rc::default();
        if query.is_empty() {
            return Ok(());
        }
        let query = if regular_expression {
            query.to_owned()
        } else {
            regex::escape(query)
        };
        let query = if whole_word {
            format!(r"\b(?:{query})\b")
        } else {
            query
        };
        self.expression = Some(
            RegexBuilder::new(&query)
                .case_insensitive(!case_sensitive)
                .multi_line(true)
                .build()?,
        );
        self.collect();
        Ok(())
    }
    fn collect(&mut self) {
        self.ranges = Rc::new(
            self.expression
                .as_ref()
                .map(|expression| {
                    expression
                        .find_iter(&self.text)
                        .map(|matched| matched.range())
                        .collect()
                })
                .unwrap_or_default(),
        );
        self.current = self.current.min(self.ranges.len().saturating_sub(1));
    }
    pub fn matched_ranges(&self) -> Rc<Vec<Range<usize>>> {
        self.ranges.clone()
    }
    pub fn current(&self) -> Option<usize> {
        (!self.ranges.is_empty()).then_some(self.current)
    }
    pub fn is_empty(&self) -> bool {
        self.ranges.is_empty()
    }
    pub fn label(&self) -> String {
        format!(
            "{} / {}",
            self.current().map_or(0, |current| current + 1),
            self.ranges.len()
        )
    }
    pub fn next(&mut self) {
        if !self.ranges.is_empty() {
            self.current = (self.current + 1) % self.ranges.len();
        }
    }
    pub fn next_back(&mut self) {
        if !self.ranges.is_empty() {
            self.current = (self.current + self.ranges.len() - 1) % self.ranges.len();
        }
    }
    pub fn replacement(&self, replacement: &str, all: bool) -> Option<(Range<usize>, String)> {
        let expression = self.expression.as_ref()?;
        if all {
            let result = if self.regular_expression {
                expression.replace_all(&self.text, replacement)
            } else {
                expression.replace_all(&self.text, regex::NoExpand(replacement))
            };
            return Some((0..self.text.len(), result.into_owned()));
        }
        let range = self.ranges.get(self.current()?)?.clone();
        let value = if self.regular_expression {
            let captures = expression.captures_at(&self.text, range.start)?;
            let mut result = String::new();
            captures.expand(replacement, &mut result);
            result
        } else {
            replacement.to_owned()
        };
        Some((range, value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unicode_word_case_and_regex_replacement() {
        let mut matcher = SearchMatcher::new();
        matcher.update("你好 abc ABC abcd id=12 id=34");
        matcher.update_options("abc", false, true, false).unwrap();
        assert_eq!(matcher.matched_ranges().len(), 2);
        matcher.next_back();
        assert_eq!(matcher.current(), Some(1));
        matcher.next();
        assert_eq!(matcher.current(), Some(0));
        matcher
            .update_options(r"id=(\d+)", true, false, true)
            .unwrap();
        let (_, text) = matcher.replacement("num:$1", true).unwrap();
        assert!(text.ends_with("num:12 num:34"));
        assert!(matcher.update_options("[", true, false, true).is_err());
        assert!(matcher.is_empty());
        matcher.update_options("abc", true, false, false).unwrap();
        let (_, literal) = matcher.replacement("$1", false).unwrap();
        assert_eq!(literal, "$1");
    }
}
