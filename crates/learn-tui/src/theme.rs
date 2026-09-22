use ratatui::style::{Color, Modifier, Style};

/// The visual palettes available from the command center.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ThemeId {
    Default,
    Catppuccin,
    Nord,
    TokyoNight,
    Everforest,
    EverforestLight,
}

impl ThemeId {
    pub(crate) const fn theme(self) -> Theme {
        match self {
            Self::Default => Theme {
                fg: Color::Rgb(232, 238, 245),
                dim: Color::Rgb(174, 185, 198),
                muted: Color::Rgb(118, 132, 149),
                accent: Color::Rgb(125, 211, 192),
                success: Color::Rgb(127, 184, 143),
                warn: Color::Rgb(232, 185, 92),
                error: Color::Rgb(209, 120, 120),
                selected_fg: Color::Rgb(15, 23, 32),
                selected_bg: Color::Rgb(125, 211, 192),
                surface: Color::Rgb(14, 20, 29),
                overlay: Color::Rgb(28, 40, 56),
                backdrop: Color::Rgb(42, 67, 78),
                pet_primary: Color::Rgb(235, 146, 72),
                pet_secondary: Color::Rgb(255, 195, 106),
            },
            Self::Catppuccin => Theme {
                fg: Color::Rgb(220, 226, 250),
                dim: Color::Rgb(188, 195, 220),
                muted: Color::Rgb(145, 151, 176),
                accent: Color::Rgb(137, 220, 235),
                success: Color::Rgb(166, 227, 161),
                warn: Color::Rgb(249, 226, 175),
                error: Color::Rgb(243, 139, 168),
                selected_fg: Color::Rgb(17, 17, 27),
                selected_bg: Color::Rgb(137, 220, 235),
                surface: Color::Rgb(49, 50, 68),
                overlay: Color::Rgb(69, 71, 90),
                backdrop: Color::Rgb(86, 79, 112),
                pet_primary: Color::Rgb(235, 146, 72),
                pet_secondary: Color::Rgb(255, 195, 106),
            },
            Self::Nord => Theme {
                fg: Color::Rgb(226, 232, 242),
                dim: Color::Rgb(183, 194, 211),
                muted: Color::Rgb(136, 151, 174),
                accent: Color::Rgb(136, 192, 208),
                success: Color::Rgb(163, 190, 140),
                warn: Color::Rgb(235, 203, 139),
                error: Color::Rgb(191, 97, 106),
                selected_fg: Color::Rgb(46, 52, 64),
                selected_bg: Color::Rgb(136, 192, 208),
                surface: Color::Rgb(59, 66, 82),
                overlay: Color::Rgb(67, 76, 94),
                backdrop: Color::Rgb(82, 103, 123),
                pet_primary: Color::Rgb(235, 146, 72),
                pet_secondary: Color::Rgb(255, 195, 106),
            },
            Self::TokyoNight => Theme {
                fg: Color::Rgb(215, 222, 255),
                dim: Color::Rgb(164, 174, 210),
                muted: Color::Rgb(119, 130, 171),
                accent: Color::Rgb(125, 207, 255),
                success: Color::Rgb(158, 206, 106),
                warn: Color::Rgb(224, 175, 104),
                error: Color::Rgb(247, 118, 142),
                selected_fg: Color::Rgb(26, 27, 38),
                selected_bg: Color::Rgb(125, 207, 255),
                surface: Color::Rgb(36, 40, 59),
                overlay: Color::Rgb(47, 53, 73),
                backdrop: Color::Rgb(66, 76, 112),
                pet_primary: Color::Rgb(235, 146, 72),
                pet_secondary: Color::Rgb(255, 195, 106),
            },
            Self::Everforest => Theme {
                fg: Color::Rgb(225, 214, 187),
                dim: Color::Rgb(185, 191, 166),
                muted: Color::Rgb(127, 137, 116),
                accent: Color::Rgb(167, 192, 128),
                success: Color::Rgb(167, 192, 128),
                warn: Color::Rgb(219, 188, 127),
                error: Color::Rgb(230, 126, 128),
                selected_fg: Color::Rgb(45, 53, 59),
                selected_bg: Color::Rgb(167, 192, 128),
                surface: Color::Rgb(61, 72, 77),
                overlay: Color::Rgb(77, 89, 96),
                backdrop: Color::Rgb(91, 112, 91),
                pet_primary: Color::Rgb(235, 146, 72),
                pet_secondary: Color::Rgb(255, 195, 106),
            },
            Self::EverforestLight => Theme {
                fg: Color::Rgb(92, 106, 114),
                dim: Color::Rgb(104, 119, 108),
                muted: Color::Rgb(126, 139, 119),
                accent: Color::Rgb(141, 161, 1),
                success: Color::Rgb(127, 187, 179),
                warn: Color::Rgb(223, 160, 0),
                error: Color::Rgb(248, 85, 82),
                selected_fg: Color::Rgb(253, 246, 227),
                selected_bg: Color::Rgb(141, 161, 1),
                surface: Color::Rgb(242, 239, 223),
                overlay: Color::Rgb(230, 226, 207),
                backdrop: Color::Rgb(199, 211, 174),
                pet_primary: Color::Rgb(235, 146, 72),
                pet_secondary: Color::Rgb(255, 195, 106),
            },
        }
    }
}

/// Semantic colors used throughout the TUI.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Theme {
    pub(crate) fg: Color,
    pub(crate) dim: Color,
    pub(crate) muted: Color,
    pub(crate) accent: Color,
    pub(crate) success: Color,
    pub(crate) warn: Color,
    pub(crate) error: Color,
    pub(crate) selected_fg: Color,
    pub(crate) selected_bg: Color,
    pub(crate) surface: Color,
    pub(crate) overlay: Color,
    pub(crate) backdrop: Color,
    pub(crate) pet_primary: Color,
    pub(crate) pet_secondary: Color,
}

impl Theme {
    pub(crate) fn title(self) -> Style {
        Style::default().fg(self.fg).add_modifier(Modifier::BOLD)
    }

    pub(crate) fn dim(self) -> Style {
        Style::default().fg(self.dim)
    }

    pub(crate) fn muted(self) -> Style {
        Style::default().fg(self.muted)
    }

    pub(crate) fn accent(self) -> Style {
        Style::default().fg(self.accent)
    }

    pub(crate) fn error(self) -> Style {
        Style::default().fg(self.error)
    }

    pub(crate) fn selected(self) -> Style {
        Style::default()
            .fg(self.selected_fg)
            .bg(self.selected_bg)
            .add_modifier(Modifier::BOLD)
    }
}
