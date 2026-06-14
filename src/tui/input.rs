#[derive(Debug, Clone, Copy, Default, Eq, PartialEq)]
pub(super) enum InputKind {
    #[default]
    None,
    Command,
    Filter,
    Rate,
}

#[derive(Debug, Default)]
enum InputMode {
    #[default]
    None,
    Command(String),
    Filter,
    Rate(String),
}

#[derive(Debug, Default)]
pub(super) struct InputState {
    active_filter: String,
    mode: InputMode,
}

impl InputState {
    pub(super) fn kind(&self) -> InputKind {
        match self.mode {
            InputMode::None => InputKind::None,
            InputMode::Command(_) => InputKind::Command,
            InputMode::Filter => InputKind::Filter,
            InputMode::Rate(_) => InputKind::Rate,
        }
    }

    pub(super) fn is_active(&self) -> bool {
        self.kind() != InputKind::None
    }

    pub(super) fn enter_command(&mut self) {
        self.mode = InputMode::Command(String::new());
    }

    pub(super) fn enter_filter(&mut self) {
        self.mode = InputMode::Filter;
    }

    pub(super) fn enter_rate(&mut self) {
        self.mode = InputMode::Rate(String::new());
    }

    pub(super) fn cancel_command(&mut self) {
        if matches!(self.mode, InputMode::Command(_)) {
            self.mode = InputMode::None;
        }
    }

    pub(super) fn take_command(&mut self) -> Option<String> {
        let mode = std::mem::take(&mut self.mode);
        match mode {
            InputMode::Command(command) => Some(command),
            mode => {
                self.mode = mode;
                None
            }
        }
    }

    pub(super) fn finish_filter(&mut self) {
        if matches!(self.mode, InputMode::Filter) {
            self.mode = InputMode::None;
        }
    }

    pub(super) fn clear_filter(&mut self) {
        self.active_filter.clear();
        self.finish_filter();
    }

    pub(super) fn finish_rate(&mut self) {
        if matches!(self.mode, InputMode::Rate(_)) {
            self.mode = InputMode::None;
        }
    }

    pub(super) fn command(&self) -> &str {
        match &self.mode {
            InputMode::Command(command) => command,
            InputMode::None | InputMode::Filter | InputMode::Rate(_) => "",
        }
    }

    pub(super) fn replace_command(&mut self, replacement: String) {
        if let InputMode::Command(command) = &mut self.mode {
            *command = replacement;
        }
    }

    pub(super) fn push_command(&mut self, char: char) {
        if let InputMode::Command(command) = &mut self.mode {
            command.push(char);
        }
    }

    pub(super) fn pop_command(&mut self) {
        if let InputMode::Command(command) = &mut self.mode {
            command.pop();
        }
    }

    pub(super) fn filter(&self) -> &str {
        &self.active_filter
    }

    pub(super) fn set_filter(&mut self, filter: String) {
        self.active_filter = filter;
    }

    pub(super) fn push_filter(&mut self, char: char) {
        if matches!(self.mode, InputMode::Filter) {
            self.active_filter.push(char);
        }
    }

    pub(super) fn pop_filter(&mut self) {
        if matches!(self.mode, InputMode::Filter) {
            self.active_filter.pop();
        }
    }

    pub(super) fn rate(&self) -> &str {
        match &self.mode {
            InputMode::Rate(rate) => rate,
            InputMode::None | InputMode::Command(_) | InputMode::Filter => "",
        }
    }

    pub(super) fn push_rate(&mut self, char: char) {
        if let InputMode::Rate(rate) = &mut self.mode {
            rate.push(char);
        }
    }

    pub(super) fn pop_rate(&mut self) {
        if let InputMode::Rate(rate) = &mut self.mode {
            rate.pop();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entering_modes_is_mutually_exclusive_and_preserves_filter() {
        let mut input = InputState::default();
        input.set_filter(String::from("artist:eno"));

        input.enter_filter();
        assert_eq!(input.kind(), InputKind::Filter);

        input.enter_command();
        assert_eq!(input.kind(), InputKind::Command);
        assert_eq!(input.filter(), "artist:eno");

        input.enter_rate();
        assert_eq!(input.kind(), InputKind::Rate);
        assert!(input.command().is_empty());

        input.finish_rate();
        assert_eq!(input.kind(), InputKind::None);
    }

    #[test]
    fn transient_buffers_are_owned_by_their_modes() {
        let mut input = InputState::default();
        input.enter_command();
        input.replace_command(String::from("library"));

        assert_eq!(input.take_command().as_deref(), Some("library"));
        assert_eq!(input.kind(), InputKind::None);

        input.enter_rate();
        input.push_rate('7');
        input.push_rate('5');
        input.enter_command();

        assert!(input.rate().is_empty());
        assert!(input.command().is_empty());

        input.cancel_command();

        assert_eq!(input.kind(), InputKind::None);
    }

    #[test]
    fn wrong_mode_transitions_preserve_the_active_mode() {
        let mut input = InputState::default();
        input.enter_rate();
        input.push_rate('7');
        input.push_rate('5');

        input.cancel_command();
        input.finish_filter();
        input.replace_command(String::from("library"));

        assert_eq!(input.kind(), InputKind::Rate);
        assert_eq!(input.rate(), "75");
        assert_eq!(input.take_command(), None);
        assert_eq!(input.kind(), InputKind::Rate);
        assert_eq!(input.rate(), "75");
    }
}
