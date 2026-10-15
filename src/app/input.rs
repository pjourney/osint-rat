//! Single-line text input used by forms, search, and the agent prompt.

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Input {
    pub value: String,
    pub cursor: usize,
}

impl Input {
    pub fn new(value: impl Into<String>) -> Self {
        let value = value.into();
        let cursor = value.len();
        Self { value, cursor }
    }

    fn clamp_cursor(&mut self) {
        self.cursor = Self::floor_boundary(&self.value, self.cursor);
    }

    /// Byte index at or before `index` that is a char boundary.
    pub fn floor_boundary(text: &str, index: usize) -> usize {
        let mut index = index.min(text.len());
        while index > 0 && !text.is_char_boundary(index) {
            index -= 1;
        }
        index
    }

    pub fn insert(&mut self, ch: char) {
        if ch.is_control() {
            return;
        }
        self.clamp_cursor();
        self.value.insert(self.cursor, ch);
        self.cursor += ch.len_utf8();
    }

    pub fn backspace(&mut self) {
        self.clamp_cursor();
        if self.cursor == 0 {
            return;
        }
        let previous = self.value[..self.cursor]
            .chars()
            .next_back()
            .map(|ch| ch.len_utf8())
            .unwrap_or(1);
        let start = self.cursor - previous;
        self.value.replace_range(start..self.cursor, "");
        self.cursor = start;
    }

    pub fn delete(&mut self) {
        self.clamp_cursor();
        if self.cursor >= self.value.len() {
            return;
        }
        let len = self.value[self.cursor..]
            .chars()
            .next()
            .map(|ch| ch.len_utf8())
            .unwrap_or(1);
        self.value.replace_range(self.cursor..self.cursor + len, "");
    }

    pub fn left(&mut self) {
        self.clamp_cursor();
        if self.cursor == 0 {
            return;
        }
        let previous = self.value[..self.cursor]
            .chars()
            .next_back()
            .map(|ch| ch.len_utf8())
            .unwrap_or(1);
        self.cursor -= previous;
    }

    pub fn right(&mut self) {
        self.clamp_cursor();
        if self.cursor >= self.value.len() {
            return;
        }
        let next = self.value[self.cursor..]
            .chars()
            .next()
            .map(|ch| ch.len_utf8())
            .unwrap_or(1);
        self.cursor += next;
    }

    pub fn home(&mut self) {
        self.cursor = 0;
    }

    pub fn end(&mut self) {
        self.cursor = self.value.len();
    }

    pub fn set(&mut self, value: impl Into<String>) {
        self.value = value.into();
        self.cursor = self.value.len();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inserts_and_deletes_at_cursor() {
        let mut input = Input::new("ac");
        input.home();
        input.right();
        input.insert('b');
        assert_eq!(input.value, "abc");
        input.backspace();
        assert_eq!(input.value, "ac");
    }

    #[test]
    fn utf8_and_mid_code_unit_cursor_do_not_panic() {
        let mut input = Input::new("café");
        input.home();
        input.right();
        input.right();
        input.right();
        input.insert('é');
        assert_eq!(input.value, "caféé");
        input.cursor = 4;
        input.left();
        input.insert('X');
        assert!(input.value.contains('X'));
        assert!(input.value.is_char_boundary(input.cursor));
        input.cursor = 5;
        input.backspace();
        input.cursor = 1;
        input.delete();
        input.cursor = usize::MAX;
        input.right();
        assert!(input.cursor <= input.value.len());
        assert!(input.value.is_char_boundary(input.cursor));
    }
}
