//! Terminal presentation only; search state and fingerprints stay in the core.

use clap::builder::styling::{AnsiColor, Style};
use clap::ColorChoice;
use std::fmt;

/// Resolves the color preference for one comparison output stream.
///
/// `is_terminal` describes the stdout or stderr destination; stdin is irrelevant.
/// Resolves `Auto` to `Always` only for a terminal when `NO_COLOR` is unset or
/// empty and `TERM` is not `dumb`, and to `Never` otherwise. Explicit choices
/// override those conditions. `CLICOLOR_FORCE` is deliberately ignored.
pub fn color_for_stream(choice: ColorChoice, is_terminal: bool) -> ColorChoice {
    let no_color = std::env::var_os("NO_COLOR").is_some_and(|value| !value.is_empty());
    let dumb_terminal = std::env::var_os("TERM").is_some_and(|value| value == "dumb");
    resolve_color(choice, is_terminal, no_color, dumb_terminal)
}

fn resolve_color(
    choice: ColorChoice,
    is_terminal: bool,
    no_color: bool,
    dumb_terminal: bool,
) -> ColorChoice {
    match choice {
        ColorChoice::Auto if is_terminal && !no_color && !dumb_terminal => ColorChoice::Always,
        ColorChoice::Auto => ColorChoice::Never,
        explicit => explicit,
    }
}

/// Independent palettes for ordinary output and diagnostics.
#[derive(Clone, Copy)]
pub struct Presentation {
    /// Palette for stdout.
    pub output: Palette,
    /// Palette for stderr.
    pub diagnostics: Palette,
}

/// Workflow styles enabled or disabled for a destination stream.
///
/// Each formatting helper preserves the displayed value and applies styling
/// only when this palette is enabled.
#[derive(Clone, Copy)]
pub struct Palette {
    enabled: bool,
}

impl Palette {
    /// Creates a palette from a choice resolved by [`color_for_stream`].
    ///
    /// Only `Always` enables styling. `Auto` disables it here rather than
    /// querying the terminal or environment.
    pub fn new(choice: ColorChoice) -> Self {
        Self {
            enabled: choice == ColorChoice::Always,
        }
    }

    /// Formats counts and fingerprints in bold with the default foreground.
    pub fn bold<T: fmt::Display>(self, value: T) -> Styled<T> {
        self.style(value, Style::new().bold())
    }

    /// Formats input prompts in cyan.
    pub fn prompt<T: fmt::Display>(self, value: T) -> Styled<T> {
        self.style(value, AnsiColor::Cyan.on_default())
    }

    /// Formats comparison headings in bold cyan.
    pub fn heading<T: fmt::Display>(self, value: T) -> Styled<T> {
        self.style(value, AnsiColor::Cyan.on_default().bold())
    }

    /// Formats confirmed match results in green.
    pub fn matched<T: fmt::Display>(self, value: T) -> Styled<T> {
        self.style(value, AnsiColor::Green.on_default())
    }

    /// Formats divergence results and character annotations in bold yellow.
    pub fn difference<T: fmt::Display>(self, value: T) -> Styled<T> {
        self.style(value, AnsiColor::Yellow.on_default().bold())
    }

    /// Formats diagnostic labels in red.
    pub fn error<T: fmt::Display>(self, value: T) -> Styled<T> {
        self.style(value, AnsiColor::Red.on_default())
    }

    fn style<T: fmt::Display>(self, value: T, style: Style) -> Styled<T> {
        Styled {
            value,
            style: if self.enabled { style } else { Style::new() },
        }
    }
}

/// A display value wrapped in optional ANSI styling.
///
/// Formatting emits the style, the value, and a reset before subsequent text
/// or input. With a disabled palette, formatting adds no escape sequences.
pub struct Styled<T> {
    value: T,
    style: Style,
}

impl<T: fmt::Display> fmt::Display for Styled<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{}{:#}", self.style, self.value, self.style)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn automatic_color_requires_a_terminal_and_respects_environment() {
        for terminal in [false, true] {
            for no_color in [false, true] {
                for dumb in [false, true] {
                    let expected = if terminal && !no_color && !dumb {
                        ColorChoice::Always
                    } else {
                        ColorChoice::Never
                    };
                    assert_eq!(
                        resolve_color(ColorChoice::Auto, terminal, no_color, dumb),
                        expected
                    );
                    for explicit in [ColorChoice::Always, ColorChoice::Never] {
                        assert_eq!(resolve_color(explicit, terminal, no_color, dumb), explicit);
                    }
                }
            }
        }
    }
}
