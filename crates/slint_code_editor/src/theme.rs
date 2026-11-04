use crate::syntax::Category;
use cosmic_text::Color;

/// Colors never change document geometry or enter undo history.
#[derive(Clone, Debug, PartialEq)]
pub struct Theme {
    pub background: Color,
    pub foreground: Color,
    pub gutter: Color,
    pub line_number: Color,
    pub cursor: Color,
    pub selection: Color,
    pub selected_text: Color,
    pub keyword: Color,
    pub type_name: Color,
    pub function: Color,
    pub number: Color,
    pub string: Color,
    pub comment: Color,
    pub punctuation: Color,
}
impl Default for Theme {
    fn default() -> Self {
        Self {
            background: Color::rgb(255, 255, 255),
            foreground: Color::rgb(29, 29, 31),
            gutter: Color::rgb(247, 247, 249),
            line_number: Color::rgb(130, 130, 140),
            cursor: Color::rgb(0, 100, 220),
            selection: Color::rgb(185, 215, 255),
            selected_text: Color::rgb(29, 29, 31),
            keyword: Color::rgb(150, 40, 170),
            type_name: Color::rgb(0, 110, 135),
            function: Color::rgb(30, 75, 180),
            number: Color::rgb(160, 70, 15),
            string: Color::rgb(170, 45, 45),
            comment: Color::rgb(70, 125, 75),
            punctuation: Color::rgb(80, 80, 95),
        }
    }
}
impl Theme {
    pub fn dark() -> Self {
        Self {
            background: Color::rgb(30, 31, 35),
            foreground: Color::rgb(225, 225, 230),
            gutter: Color::rgb(25, 26, 30),
            line_number: Color::rgb(145, 145, 155),
            cursor: Color::rgb(140, 185, 255),
            selection: Color::rgb(60, 80, 120),
            selected_text: Color::rgb(255, 255, 255),
            keyword: Color::rgb(205, 145, 235),
            type_name: Color::rgb(100, 205, 205),
            function: Color::rgb(140, 185, 255),
            number: Color::rgb(235, 180, 125),
            string: Color::rgb(230, 155, 145),
            comment: Color::rgb(135, 180, 130),
            punctuation: Color::rgb(190, 190, 200),
        }
    }
    pub fn color(&self, category: Category) -> Color {
        match category {
            Category::Keyword | Category::Boolean => self.keyword,
            Category::Type => self.type_name,
            Category::Attribute | Category::Function => self.function,
            Category::Number => self.number,
            Category::String => self.string,
            Category::Comment => self.comment,
            Category::Punctuation => self.punctuation,
        }
    }
}
