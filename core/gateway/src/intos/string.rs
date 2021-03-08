pub trait IndexOf {
    fn index_of(&mut self, character: char) -> Option<usize>;
}

impl IndexOf for String {
    fn index_of(&mut self, character: char) -> Option<usize> {
        self.chars().position(|c| c == character)
    }
}