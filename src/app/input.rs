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

    pub fn insert(&mut self, ch: char) {
        if ch.is_control() {
            return;
        }
        self.value.insert(self.cursor, ch);
        self.cursor += ch.len_utf8();
    }

    pub fn backspace(&mut self) {
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
}
