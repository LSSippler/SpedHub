//! Aussehen des Launchers nach `docs/design-system.md`: Farben, Schrift, Abstände und die
//! Bausteine (Karte, Plakette, Kennzahl, Hinweis, Kennzeichen, Schadensbalken, Knöpfe,
//! Seitenleiste).
//!
//! Die Farben tragen dieselben Namen und Werte wie die CSS-Variablen im Dashboard (`--bg` heißt
//! hier `BG`, `--success-soft` heißt `SUCCESS_SOFT`). Feste Farben, Größen oder Abstände gibt es
//! außerhalb dieses Moduls nicht. Was sich ohne Fenster prüfen lässt (Kontrast, Farbe des
//! Schadensbalkens, Zahlen und Namen formatieren), steht in den Tests unten.

use eframe::egui::{
    self, Align, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Frame,
    Layout, Margin, Painter, Rect, Response, Sense, Shadow, Stroke, StrokeKind, Style, TextEdit,
    TextStyle, Theme, ThemePreference, Ui, Vec2, Visuals, Widget, WidgetText, pos2,
    style::{Selection, WidgetVisuals, Widgets},
    vec2,
};
use std::sync::Arc;

/// Farbe aus der Schreibweise wie im CSS: `rgb(0xf2f4f7)` ist `#f2f4f7`.
const fn rgb(hex: u32) -> Color32 {
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

macro_rules! tokens {
    ($($(#[$doc:meta])* $name:ident $field:ident = $light:literal, $dark:literal;)*) => {
        /// Farben im hellen Modus, wie `:root` im Dashboard.
        pub mod light {
            use super::{Color32, rgb};
            $($(#[$doc])* pub const $name: Color32 = rgb($light);)*
        }

        /// Farben im dunklen Modus, wie `prefers-color-scheme: dark` im Dashboard.
        pub mod dark {
            use super::{Color32, rgb};
            $($(#[$doc])* pub const $name: Color32 = rgb($dark);)*
        }

        /// Alle Farben eines Modus. [`palette`] liefert die passende zur Oberfläche.
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub struct Palette {
            $($(#[$doc])* pub $field: Color32,)*
        }

        pub const LIGHT: Palette = Palette { $($field: light::$name,)* };
        pub const DARK: Palette = Palette { $($field: dark::$name,)* };
    };
}

tokens! {
    /// Seitengrund
    BG bg = 0xf2f4f7, 0x0b1220;
    /// Karten, Tabellen, Dialoge
    SURFACE surface = 0xffffff, 0x121b2c;
    /// Eingelassene Flächen, Kopf von Tabellen
    SURFACE_2 surface_2 = 0xf8fafc, 0x0f1727;
    /// Linien, Ränder von Karten
    BORDER border = 0xe3e8ef, 0x223049;
    /// Ränder von Eingaben
    BORDER_STRONG border_strong = 0xcbd5e1, 0x33425e;
    /// Text
    TEXT text = 0x0f172a, 0xe5eaf2;
    /// Nebentext, Beschriftungen
    TEXT_2 text_2 = 0x475569, 0xa3b1c6;
    /// Hinweise, leere Zustände
    TEXT_3 text_3 = 0x64748b, 0x8191a8;
    /// Hauptaktion, Links, Auswahl
    PRIMARY primary = 0x1d4ed8, 0x5b8def;
    PRIMARY_HOVER primary_hover = 0x1e40af, 0x7aa3f5;
    /// Hintergrund ausgewählter Zeilen, aktive Navigation
    PRIMARY_SOFT primary_soft = 0xe6eefc, 0x17284a;
    /// Schrift auf `PRIMARY` und `DANGER`. Im Dashboard weiß, im dunklen Modus hätte Weiß auf
    /// dem hellen Blau nur 3,2 zu 1 Kontrast, deshalb dort die Farbe des Seitengrunds.
    ON_PRIMARY on_primary = 0xffffff, 0x0b1220;
    /// Signalfarbe der Marke: Markierung der aktiven Navigation, Logo
    ACCENT accent = 0xea7a1a, 0xf0953f;
    /// frei, bezahlt, online, Plus
    SUCCESS success = 0x166534, 0x4ade80;
    SUCCESS_SOFT success_soft = 0xe3f6ea, 0x123423;
    /// unterwegs, verliehen, Achtung
    WARNING warning = 0xa34a06, 0xfbbf24;
    WARNING_SOFT warning_soft = 0xfdf1dc, 0x3a2a0c;
    /// Werkstatt, Minus, Fehler
    DANGER danger = 0xb91c1c, 0xf87171;
    DANGER_SOFT danger_soft = 0xfde7e7, 0x3b1515;
    /// neutrale Hinweise
    INFO info = 0x0e7490, 0x38bdf8;
    INFO_SOFT info_soft = 0xe0f3f8, 0x0d2c3a;
    /// Seitenleiste
    NAV_BG nav_bg = 0x0f1e33, 0x0a1424;
    NAV_TEXT nav_text = 0xc3cede, 0xaab7ca;
    NAV_ACTIVE nav_active = 0xffffff, 0xffffff;
    NAV_ACTIVE_BG nav_active_bg = 0x1b3150, 0x162843;
}

/// Das Kennzeichen sieht in beiden Modi aus wie ein echtes Schild.
pub const PLATE_BG: Color32 = rgb(0xffffff);
pub const PLATE_TEXT: Color32 = rgb(0x14181b);
pub const PLATE_BLUE: Color32 = rgb(0x0b3d91);

// Abstände im 4er Raster, `--sp-1` bis `--sp-10`
pub const SP_1: f32 = 4.0;
pub const SP_2: f32 = 8.0;
pub const SP_3: f32 = 12.0;
pub const SP_4: f32 = 16.0;
pub const SP_5: f32 = 20.0;
pub const SP_6: f32 = 24.0;
pub const SP_8: f32 = 32.0;
pub const SP_10: f32 = 40.0;

// Radien
pub const R_SM: u8 = 6;
pub const R_MD: u8 = 10;
pub const R_PILL: u8 = 255;

// Schriftgrößen, `--fs-xs` bis `--fs-xl`
pub const FS_XS: f32 = 12.0;
pub const FS_SM: f32 = 13.0;
pub const FS_BASE: f32 = 14.0;
pub const FS_MD: f32 = 16.0;
pub const FS_LG: f32 = 20.0;
pub const FS_XL: f32 = 28.0;

/// Breite der Seitenleiste
pub const NAV_WIDTH: f32 = 224.0;
/// Höhe der Knöpfe und Eingaben
pub const CONTROL_HEIGHT: f32 = 32.0;
/// Kleinste Breite einer Lkw-Karte. Breiter als die 320 px im Dashboard, weil die eigene Leihe
/// zwei Knöpfe nebeneinander hat ("Zurückgeben" und "In Spielstand schreiben", zusammen mit
/// Innenabstand rund 366 px). egui bricht die Knopfreihe nicht um, eine schmalere Karte würde
/// breiter als ihre Spalte und die Nachbarkarte aus dem Fenster schieben.
pub const CARD_MIN_WIDTH: f32 = 380.0;
/// Größte Breite des Inhalts
pub const CONTENT_MAX_WIDTH: f32 = 1280.0;

/// Name der Schriftfamilie für Gewicht 600 (Titel, Knöpfe, Kennzahlen).
pub const SEMIBOLD: &str = "semibold";

/// Umgebungsvariable, die hell oder dunkel erzwingt (`hell` oder `dunkel`). Sonst gilt die
/// Systemeinstellung.
pub const THEME_VAR: &str = "SPEDHUB_THEME";

// ---------------------------------------------------------------------- Einrichten

/// Schrift mit Gewicht 400 in `size`.
pub fn regular(size: f32) -> FontId {
    FontId::proportional(size)
}

/// Schrift mit Gewicht 600 in `size`.
pub fn semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(SEMIBOLD.into()))
}

/// Lässt sich mit dieser Schriftdatei Text auslegen? egui bricht mit einem Panic ab, wenn eine
/// Datei nicht als TTF/OTF lesbar ist, und zwar erst beim ersten Zeichnen. Eine leere oder
/// kaputte Systemschrift würde den Launcher also beim Start ohne Meldung schließen. Deshalb
/// ein Probelauf in einem eigenen Kontext, bevor die Schrift genommen wird.
pub fn font_works(data: &[u8]) -> bool {
    if data.is_empty() {
        return false;
    }
    let data = data.to_vec();
    std::panic::catch_unwind(move || {
        let ctx = egui::Context::default();
        let mut fonts = FontDefinitions::default();
        fonts
            .font_data
            .insert("probe".into(), Arc::new(FontData::from_owned(data)));
        if let Some(family) = fonts.families.get_mut(&FontFamily::Proportional) {
            family.insert(0, "probe".into());
        }
        ctx.set_fonts(fonts);
        for _ in 0..2 {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                ui.label("Probe: Lkw Äöü ß 1.234 €");
            });
            // Ohne Grafikkarte gibt es niemanden, der die Texturen abnimmt
            out.textures_delta.clear();
        }
    })
    .is_ok()
}

/// Schriften: die Systemschrift (unter Windows Segoe UI), falls sie sich lesen ließ und der
/// Probelauf in [`font_works`] klappt, sonst die eingebaute von egui. Die Familie [`SEMIBOLD`]
/// gibt es immer, notfalls mit der normalen Schrift.
pub fn font_definitions(regular: Option<Vec<u8>>, semibold: Option<Vec<u8>>) -> FontDefinitions {
    let regular = regular.filter(|d| font_works(d));
    let semibold = semibold.filter(|d| font_works(d));
    let mut fonts = FontDefinitions::default();
    let fallback = fonts
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    let mut proportional = fallback.clone();
    if let Some(data) = regular {
        fonts
            .font_data
            .insert("system".into(), Arc::new(FontData::from_owned(data)));
        proportional.insert(0, "system".into());
    }
    let mut strong = proportional.clone();
    if let Some(data) = semibold {
        fonts.font_data.insert(
            "system-semibold".into(),
            Arc::new(FontData::from_owned(data)),
        );
        strong.insert(0, "system-semibold".into());
    }
    fonts
        .families
        .insert(FontFamily::Proportional, proportional);
    fonts
        .families
        .insert(FontFamily::Name(SEMIBOLD.into()), strong);
    fonts
}

/// Hell, dunkel oder nach System, aus dem Wert von [`THEME_VAR`].
pub fn theme_preference(value: Option<&str>) -> ThemePreference {
    match value.map(|v| v.trim().to_lowercase()).as_deref() {
        Some("hell" | "light") => ThemePreference::Light,
        Some("dunkel" | "dark") => ThemePreference::Dark,
        _ => ThemePreference::System,
    }
}

/// Setzt Schrift, Farben und Abstände für hell und dunkel. Welcher Modus gilt, entscheidet
/// `preference`, bei [`ThemePreference::System`] das Betriebssystem. Meldet es nichts (etwa
/// unter X11), bleibt es hell.
pub fn install(ctx: &egui::Context, fonts: FontDefinitions, preference: ThemePreference) {
    ctx.set_fonts(fonts);
    ctx.set_style_of(Theme::Light, style(&LIGHT, false));
    ctx.set_style_of(Theme::Dark, style(&DARK, true));
    ctx.options_mut(|o| {
        o.theme_preference = preference;
        o.fallback_theme = Theme::Light;
    });
}

/// Die Farben, die zur Oberfläche gehören.
pub fn palette(ui: &Ui) -> &'static Palette {
    if ui.visuals().dark_mode {
        &DARK
    } else {
        &LIGHT
    }
}

fn shadow(offset: i8, blur: u8, color: Color32) -> Shadow {
    Shadow {
        offset: [0, offset],
        blur,
        spread: 0,
        color,
    }
}

/// `--shadow-sm` für Karten. Im dunklen Modus trägt der Rand, kein Schatten.
fn shadow_sm(dark: bool) -> Shadow {
    if dark {
        Shadow::NONE
    } else {
        shadow(1, 2, Color32::from_rgba_unmultiplied(15, 23, 42, 15))
    }
}

/// `--shadow-md` für Menüs
fn shadow_md(dark: bool) -> Shadow {
    let alpha = if dark { 110 } else { 26 };
    shadow(4, 12, Color32::from_rgba_unmultiplied(15, 23, 42, alpha))
}

/// `--shadow-lg` für Dialoge
fn shadow_lg(dark: bool) -> Shadow {
    let alpha = if dark { 140 } else { 46 };
    shadow(16, 40, Color32::from_rgba_unmultiplied(15, 23, 42, alpha))
}

fn widget(fill: Color32, stroke: Color32, text: Color32) -> WidgetVisuals {
    WidgetVisuals {
        bg_fill: fill,
        weak_bg_fill: fill,
        bg_stroke: Stroke::new(1.0, stroke),
        corner_radius: CornerRadius::same(R_SM),
        fg_stroke: Stroke::new(1.0, text),
        expansion: 0.0,
    }
}

/// Stil eines Modus: Flächen, Ränder, Rundung, Abstände, Schriftgrößen, Auswahl in `primary`.
pub fn style(p: &Palette, dark: bool) -> Style {
    let mut s = Style::default();
    let mut v = if dark {
        Visuals::dark()
    } else {
        Visuals::light()
    };
    v.panel_fill = p.bg;
    v.window_fill = p.surface;
    v.extreme_bg_color = p.surface;
    v.text_edit_bg_color = Some(p.surface);
    v.faint_bg_color = p.surface_2;
    v.code_bg_color = p.surface_2;
    v.window_stroke = Stroke::new(1.0, p.border);
    v.window_corner_radius = CornerRadius::same(R_MD);
    v.menu_corner_radius = CornerRadius::same(R_MD);
    v.window_shadow = shadow_lg(dark);
    v.popup_shadow = shadow_md(dark);
    v.hyperlink_color = p.primary;
    v.warn_fg_color = p.warning;
    v.error_fg_color = p.danger;
    v.weak_text_color = Some(p.text_2);
    v.override_text_color = None;
    v.selection = Selection {
        bg_fill: p.primary_soft,
        stroke: Stroke::new(1.0, p.primary),
    };
    v.text_cursor.stroke = Stroke::new(2.0, p.primary);
    v.widgets = Widgets {
        noninteractive: widget(p.surface, p.border, p.text),
        inactive: widget(p.surface, p.border_strong, p.text),
        hovered: widget(p.surface_2, p.text_3, p.text),
        active: widget(p.border, p.primary, p.text),
        open: widget(p.surface_2, p.border_strong, p.text),
    };
    v.window_highlight_topmost = false;
    v.collapsing_header_frame = false;
    v.indent_has_left_vline = false;
    s.visuals = v;

    let sp = &mut s.spacing;
    sp.item_spacing = vec2(SP_2, SP_2);
    sp.button_padding = vec2(SP_3, 6.0);
    sp.interact_size = vec2(40.0, CONTROL_HEIGHT);
    sp.window_margin = Margin::same(SP_4 as i8);
    sp.menu_margin = Margin::same(SP_1 as i8);
    sp.indent = SP_4;
    sp.icon_width = 16.0;
    sp.icon_width_inner = 8.0;
    sp.icon_spacing = SP_2;
    sp.combo_width = 240.0;
    sp.text_edit_width = 280.0;
    sp.menu_width = 260.0;

    s.text_styles = [
        (TextStyle::Small, regular(FS_XS)),
        (TextStyle::Body, regular(FS_BASE)),
        (TextStyle::Button, regular(FS_BASE)),
        (TextStyle::Heading, semibold(FS_LG)),
        (TextStyle::Monospace, FontId::monospace(FS_SM)),
    ]
    .into();
    s
}

// ---------------------------------------------------------------------- Kontrast

/// Relative Leuchtdichte nach WCAG 2.1.
pub fn luminance(c: Color32) -> f64 {
    let channel = |v: u8| {
        let v = f64::from(v) / 255.0;
        if v <= 0.03928 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * channel(c.r()) + 0.7152 * channel(c.g()) + 0.0722 * channel(c.b())
}

/// Kontrast zweier Farben nach WCAG 2.1, von 1 bis 21.
pub fn contrast(a: Color32, b: Color32) -> f64 {
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

// ---------------------------------------------------------------------- Formatieren

/// Ganze Zahl mit Punkt als Tausendertrenner: `120.500`, `-1.234`.
pub fn thousands(n: i64) -> String {
    let digits = n.unsigned_abs().to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push('.');
        }
        out.push(c);
    }
    if n < 0 { format!("-{out}") } else { out }
}

/// Betrag in Euro: `12.345 €`.
pub fn euro(n: i64) -> String {
    format!("{} €", thousands(n))
}

/// Kilometer: `120.500 km`.
pub fn km(n: i64) -> String {
    format!("{} km", thousands(n))
}

/// Kürzel, die im Modellnamen ganz groß stehen, zusätzlich zu Teilen mit höchstens zwei
/// Buchstaben. Gleiche Liste wie `MODELL_KUERZEL` im Dashboard.
const MODEL_ABBREVIATIONS: [&str; 6] = ["daf", "man", "tgx", "tgs", "tgm", "tgl"];

/// Modell lesbar, mit denselben Regeln wie `modellName` im Dashboard: `daf.xf_euro6` wird zu
/// `DAF XF Euro 6`, `volvo.fh16_2012` zu `Volvo FH16 2012`, `mercedes.actros2014` zu
/// `Mercedes Actros 2014`. Ein Wort mit mindestens vier Buchstaben und Zahl dahinter wird
/// getrennt, Mischungen aus Buchstaben und Ziffern und kurze Kürzel stehen groß.
pub fn model_name(model: &str) -> String {
    model
        .split(['.', '_', ' '])
        .filter(|w| !w.is_empty())
        .flat_map(|w| {
            let letters = w.chars().take_while(char::is_ascii_alphabetic).count();
            let rest = &w[letters..];
            if letters >= 4 && !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()) {
                vec![&w[..letters], rest]
            } else {
                vec![w]
            }
        })
        .map(|w| {
            let has_digit = w.chars().any(|c| c.is_ascii_digit());
            let has_letter = w.chars().any(|c| c.is_ascii_alphabetic());
            let short = w.chars().count() <= 2 && w.chars().all(|c| c.is_ascii_alphabetic());
            let known = MODEL_ABBREVIATIONS.contains(&w.to_ascii_lowercase().as_str());
            if (has_digit && has_letter) || short || known {
                w.to_uppercase()
            } else {
                let mut c = w.chars();
                c.next()
                    .map(|f| f.to_uppercase().chain(c).collect())
                    .unwrap_or_default()
            }
        })
        .collect::<Vec<String>>()
        .join(" ")
}

/// Initialen für den Kreis: `Anna Berg` wird `AB`, `Chefin` wird `CH`.
pub fn initials(name: &str) -> String {
    let words: Vec<&str> = name.split_whitespace().collect();
    let letters: String = match words.as_slice() {
        [] => "?".into(),
        [one] => one.chars().take(2).collect(),
        [first, .., last] => first.chars().take(1).chain(last.chars().take(1)).collect(),
    };
    letters.to_uppercase()
}

/// Spalten für Karten mit `min_width` und Lücke `gap` in `available` Breite, und die Breite
/// einer Karte.
pub fn grid_columns(available: f32, min_width: f32, gap: f32) -> (usize, f32) {
    let columns = (((available + gap) / (min_width + gap)).floor() as usize).max(1);
    let width = ((available - gap * (columns - 1) as f32) / columns as f32).max(0.0);
    (columns, width)
}

// ---------------------------------------------------------------------- Bedeutung

/// Bedeutung einer Farbe: grün gut, Bernstein Achtung, rot Gefahr, blau Aktion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Success,
    Warning,
    Danger,
    Info,
    Primary,
    Neutral,
}

impl Tone {
    /// Kräftige Farbe für Schrift, Punkt, Symbol und Rand
    pub fn strong(self, p: &Palette) -> Color32 {
        match self {
            Tone::Success => p.success,
            Tone::Warning => p.warning,
            Tone::Danger => p.danger,
            Tone::Info => p.info,
            Tone::Primary => p.primary,
            Tone::Neutral => p.text_2,
        }
    }

    /// Weiche Farbe für Flächen
    pub fn soft(self, p: &Palette) -> Color32 {
        match self {
            Tone::Success => p.success_soft,
            Tone::Warning => p.warning_soft,
            Tone::Danger => p.danger_soft,
            Tone::Info => p.info_soft,
            Tone::Primary => p.primary_soft,
            Tone::Neutral => p.surface_2,
        }
    }

    /// Passendes Symbol für Hinweise
    pub fn icon(self) -> Icon {
        match self {
            Tone::Success => Icon::Check,
            Tone::Warning => Icon::Warning,
            Tone::Danger => Icon::Error,
            Tone::Info | Tone::Primary | Tone::Neutral => Icon::Info,
        }
    }
}

/// Farbe des Schadensbalkens: grün unter 10 %, Bernstein bis zur Schadensgrenze, rot darüber.
pub fn damage_tone(percent: f64, limit: f64) -> Tone {
    if percent > limit {
        Tone::Danger
    } else if percent < 10.0 {
        Tone::Success
    } else {
        Tone::Warning
    }
}

// ---------------------------------------------------------------------- Symbole

/// Eigene, einfache Liniensymbole im 24er Raster. Keine Logos von Herstellern.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    Truck,
    Money,
    Wallet,
    Save,
    Import,
    Settings,
    Plugin,
    Profile,
    Warning,
    Check,
    Info,
    Error,
    Speed,
    Clock,
    Refresh,
    Play,
    Logout,
    Key,
    Server,
    Download,
    ChevronDown,
    ChevronRight,
    Building,
}

/// Punkte auf einem Kreisbogen im 24er Raster, Winkel in Grad, 0 rechts, im Uhrzeigersinn.
fn arc(c: (f32, f32), r: f32, from: f32, to: f32) -> Vec<(f32, f32)> {
    let steps = (((to - from).abs() / 10.0).ceil() as usize).max(2);
    (0..=steps)
        .map(|i| {
            let a = (from + (to - from) * i as f32 / steps as f32).to_radians();
            (c.0 + r * a.cos(), c.1 + r * a.sin())
        })
        .collect()
}

/// Zeichnet `icon` in `rect` mit Strichen in `color`.
pub fn paint_icon(painter: &Painter, rect: Rect, icon: Icon, color: Color32) {
    let s = rect.width().min(rect.height()) / 24.0;
    let o = rect.center() - vec2(12.0 * s, 12.0 * s);
    let pt = |x: f32, y: f32| o + vec2(x * s, y * s);
    let stroke = Stroke::new((1.75 * s).max(1.2), color);
    let line = |pts: &[(f32, f32)]| {
        painter.line(pts.iter().map(|&(x, y)| pt(x, y)).collect(), stroke);
    };
    let circle = |x: f32, y: f32, r: f32| painter.circle_stroke(pt(x, y), r * s, stroke);
    let dot = |x: f32, y: f32, r: f32| painter.circle_filled(pt(x, y), r * s, color);
    let rect_at = |x0: f32, y0: f32, x1: f32, y1: f32, r: f32| {
        painter.rect_stroke(
            Rect::from_min_max(pt(x0, y0), pt(x1, y1)),
            CornerRadius::same((r * s) as u8),
            stroke,
            StrokeKind::Middle,
        );
    };
    match icon {
        Icon::Truck => {
            rect_at(2.0, 5.5, 14.5, 16.5, 1.0);
            line(&[
                (14.5, 9.0),
                (18.5, 9.0),
                (22.0, 12.5),
                (22.0, 16.5),
                (14.5, 16.5),
            ]);
            circle(6.5, 18.0, 2.0);
            circle(17.5, 18.0, 2.0);
        }
        Icon::Money => {
            rect_at(2.5, 6.0, 21.5, 18.0, 2.0);
            circle(12.0, 12.0, 2.8);
            dot(6.0, 12.0, 0.9);
            dot(18.0, 12.0, 0.9);
        }
        Icon::Wallet => {
            rect_at(3.0, 6.0, 21.0, 19.0, 2.5);
            line(&[(6.0, 6.0), (16.0, 3.5), (17.0, 6.0)]);
            rect_at(15.0, 10.5, 21.0, 14.5, 1.0);
            dot(17.5, 12.5, 0.9);
        }
        Icon::Save => {
            line(&[
                (4.0, 4.0),
                (16.5, 4.0),
                (20.0, 7.5),
                (20.0, 20.0),
                (4.0, 20.0),
                (4.0, 4.0),
            ]);
            line(&[(8.0, 4.0), (8.0, 8.5), (15.0, 8.5), (15.0, 4.0)]);
            rect_at(7.5, 13.0, 16.5, 20.0, 0.0);
        }
        Icon::Import | Icon::Download => {
            line(&[(12.0, 3.5), (12.0, 14.5)]);
            line(&[(7.5, 10.0), (12.0, 14.5), (16.5, 10.0)]);
            line(&[(4.0, 15.0), (4.0, 20.0), (20.0, 20.0), (20.0, 15.0)]);
        }
        Icon::Settings => {
            circle(12.0, 12.0, 3.0);
            circle(12.0, 12.0, 7.0);
            for i in 0..8 {
                let a = (i as f32 * 45.0).to_radians();
                line(&[
                    (12.0 + 7.0 * a.cos(), 12.0 + 7.0 * a.sin()),
                    (12.0 + 9.5 * a.cos(), 12.0 + 9.5 * a.sin()),
                ]);
            }
        }
        Icon::Plugin => {
            line(&[(9.0, 3.0), (9.0, 7.0)]);
            line(&[(15.0, 3.0), (15.0, 7.0)]);
            line(&[
                (6.0, 7.0),
                (18.0, 7.0),
                (18.0, 11.0),
                (15.0, 15.0),
                (9.0, 15.0),
                (6.0, 11.0),
                (6.0, 7.0),
            ]);
            line(&[(12.0, 15.0), (12.0, 21.0)]);
        }
        Icon::Profile => {
            circle(12.0, 8.0, 4.0);
            line(&arc((12.0, 21.0), 7.5, 180.0, 360.0));
        }
        Icon::Warning => {
            line(&[(12.0, 3.0), (22.0, 20.5), (2.0, 20.5), (12.0, 3.0)]);
            line(&[(12.0, 9.5), (12.0, 14.0)]);
            dot(12.0, 17.2, 1.15);
        }
        Icon::Check => {
            circle(12.0, 12.0, 9.5);
            line(&[(7.5, 12.5), (10.5, 15.5), (16.5, 9.0)]);
        }
        Icon::Info => {
            circle(12.0, 12.0, 9.5);
            line(&[(12.0, 11.0), (12.0, 16.5)]);
            dot(12.0, 7.8, 1.15);
        }
        Icon::Error => {
            circle(12.0, 12.0, 9.5);
            line(&[(9.0, 9.0), (15.0, 15.0)]);
            line(&[(15.0, 9.0), (9.0, 15.0)]);
        }
        Icon::Speed => {
            line(&arc((12.0, 15.0), 8.5, 180.0, 360.0));
            line(&[(12.0, 15.0), (16.0, 9.5)]);
            dot(12.0, 15.0, 1.3);
        }
        Icon::Clock => {
            circle(12.0, 12.0, 9.5);
            line(&[(12.0, 6.5), (12.0, 12.0), (15.5, 14.0)]);
        }
        Icon::Refresh => {
            line(&arc((12.0, 12.0), 8.0, -60.0, 230.0));
            line(&[(16.0, 2.5), (16.0, 6.6), (20.0, 6.6)]);
        }
        Icon::Play => {
            line(&[(8.0, 5.0), (19.0, 12.0), (8.0, 19.0), (8.0, 5.0)]);
        }
        Icon::Logout => {
            line(&[(10.0, 4.0), (5.0, 4.0), (5.0, 20.0), (10.0, 20.0)]);
            line(&[(10.0, 12.0), (20.5, 12.0)]);
            line(&[(16.5, 8.0), (20.5, 12.0), (16.5, 16.0)]);
        }
        Icon::Key => {
            circle(7.5, 12.0, 3.5);
            line(&[(11.0, 12.0), (21.0, 12.0)]);
            line(&[(17.5, 12.0), (17.5, 15.5)]);
            line(&[(21.0, 12.0), (21.0, 15.0)]);
        }
        Icon::Server => {
            rect_at(4.0, 4.0, 20.0, 10.5, 1.5);
            rect_at(4.0, 13.5, 20.0, 20.0, 1.5);
            dot(7.5, 7.25, 1.0);
            dot(7.5, 16.75, 1.0);
        }
        Icon::ChevronDown => line(&[(7.0, 9.5), (12.0, 14.5), (17.0, 9.5)]),
        Icon::ChevronRight => line(&[(9.5, 7.0), (14.5, 12.0), (9.5, 17.0)]),
        Icon::Building => {
            rect_at(4.0, 3.5, 14.0, 20.5, 1.0);
            line(&[(14.0, 9.0), (20.0, 9.0), (20.0, 20.5), (14.0, 20.5)]);
            for y in [7.5, 11.5, 15.5] {
                line(&[(7.0, y), (11.0, y)]);
            }
        }
    }
}

/// Symbol als eigenes Element in der Zeile.
pub fn icon(ui: &mut Ui, icon: Icon, size: f32, color: Color32) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    if ui.is_rect_visible(rect) {
        paint_icon(ui.painter(), rect, icon, color);
    }
    response
}

/// Logo: ein kleiner Lkw in `accent`, gefüllt, auf der Farbe `ground`.
pub fn paint_logo(painter: &Painter, rect: Rect, accent: Color32, ground: Color32) {
    let s = rect.width().min(rect.height()) / 24.0;
    let o = rect.center() - vec2(12.0 * s, 12.0 * s);
    let pt = |x: f32, y: f32| o + vec2(x * s, y * s);
    let r = |x0: f32, y0: f32, x1: f32, y1: f32| Rect::from_min_max(pt(x0, y0), pt(x1, y1));
    // Aufbau und Fahrerhaus
    painter.rect_filled(
        r(1.0, 4.5, 14.0, 16.5),
        CornerRadius::same((1.5 * s) as u8),
        accent,
    );
    painter.add(egui::Shape::convex_polygon(
        vec![
            pt(15.0, 8.0),
            pt(19.0, 8.0),
            pt(23.0, 12.5),
            pt(23.0, 16.5),
            pt(15.0, 16.5),
        ],
        accent,
        Stroke::NONE,
    ));
    // Fenster
    painter.add(egui::Shape::convex_polygon(
        vec![pt(16.5, 9.5), pt(18.4, 9.5), pt(20.8, 12.3), pt(16.5, 12.3)],
        ground,
        Stroke::NONE,
    ));
    // Räder mit Abstand zum Aufbau
    for x in [5.5, 18.5] {
        painter.circle_filled(pt(x, 18.0), 3.2 * s, ground);
        painter.circle_filled(pt(x, 18.0), 2.2 * s, accent);
    }
}

// ---------------------------------------------------------------------- Bausteine

/// Karte: Fläche `surface`, feiner Rand, `r_md`, kleiner Schatten.
pub fn card(ui: &Ui) -> Frame {
    let p = palette(ui);
    Frame::new()
        .fill(p.surface)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(CornerRadius::same(R_MD))
        .inner_margin(Margin::same(SP_4 as i8))
        .shadow(shadow_sm(ui.visuals().dark_mode))
}

/// Eingelassene Fläche in einer Karte (`surface-2`).
pub fn inset(ui: &Ui) -> Frame {
    let p = palette(ui);
    Frame::new()
        .fill(p.surface_2)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(CornerRadius::same(R_SM))
        .inner_margin(Margin::symmetric(SP_3 as i8, SP_2 as i8))
}

/// Text in Größe, Gewicht und Farbe.
pub fn text(text: impl Into<String>, font: FontId, color: Color32) -> WidgetText {
    egui::RichText::new(text).font(font).color(color).into()
}

/// Kopf eines Bereichs: Titel, eine Zeile Nebentext, rechts die Aktionen. Wird es eng, wird der
/// Nebentext gekürzt, nicht überdeckt.
pub fn page_header(ui: &mut Ui, title: &str, subtitle: &str, actions: impl FnOnce(&mut Ui)) {
    let p = palette(ui);
    ui.allocate_ui_with_layout(
        vec2(ui.available_width(), 48.0),
        Layout::right_to_left(Align::Center),
        |ui| {
            actions(ui);
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    ui.add(egui::Label::new(text(title, semibold(FS_LG), p.text)).truncate());
                    if !subtitle.is_empty() {
                        ui.add(
                            egui::Label::new(text(subtitle, regular(FS_SM), p.text_2)).truncate(),
                        );
                    }
                });
            });
        },
    );
}

/// Titel einer Karte mit optionaler Zeile darunter.
pub fn card_title(ui: &mut Ui, title: &str, subtitle: &str) {
    let p = palette(ui);
    ui.label(text(title, semibold(FS_MD), p.text));
    if !subtitle.is_empty() {
        ui.add(egui::Label::new(text(subtitle, regular(FS_SM), p.text_2)).wrap());
    }
}

/// Beschriftung über einem Feld.
pub fn field_label(ui: &mut Ui, label: &str) {
    let p = palette(ui);
    ui.label(text(label, semibold(FS_SM), p.text));
}

/// Hilfetext unter einem Feld.
pub fn help_text(ui: &mut Ui, help: &str) {
    let p = palette(ui);
    ui.add(egui::Label::new(text(help, regular(FS_SM), p.text_3)).wrap());
}

/// Einzeiliges Eingabefeld in Knopfhöhe.
pub fn text_field(value: &mut String) -> TextEdit<'_> {
    TextEdit::singleline(value)
        .margin(Margin::symmetric(SP_2 as i8, 7))
        .min_size(vec2(0.0, CONTROL_HEIGHT))
}

/// Plakette: Pille mit Punkt, Schrift und Punkt in der kräftigen Farbe, Fläche weich.
pub fn badge(ui: &mut Ui, label: &str, tone: Tone) -> Response {
    let p = palette(ui);
    let color = tone.strong(p);
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_string(), semibold(FS_XS), color);
    let size = vec2(galley.size().x + 8.0 + 6.0 + 6.0 + 8.0, 22.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        painter.rect_filled(rect, CornerRadius::same(R_PILL), tone.soft(p));
        painter.circle_filled(pos2(rect.left() + 11.0, rect.center().y), 3.0, color);
        let pos = pos2(rect.left() + 20.0, rect.center().y - galley.size().y / 2.0);
        painter.galley(pos, galley, color);
    }
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, label));
    response
}

/// Eine Kennzahl für [`kpi_tile`].
pub struct Kpi<'a> {
    pub icon: Icon,
    pub tone: Tone,
    pub label: &'a str,
    pub value: String,
    /// Farbe des Werts, sonst `text`
    pub value_color: Option<Color32>,
    /// Kleiner Wert (Text statt Zahl), dann in `fs-lg`
    pub small_value: bool,
    pub note: String,
}

/// Kennzahl-Kachel: Symbol im weichen Farbfeld, Beschriftung, Wert groß, eine Zeile Zusatz.
pub fn kpi_tile(ui: &mut Ui, kpi: &Kpi<'_>) -> Response {
    let p = palette(ui);
    card(ui)
        .inner_margin(Margin::same(SP_4 as i8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = SP_1;
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(Vec2::splat(32.0), Sense::hover());
                ui.painter()
                    .rect_filled(rect, CornerRadius::same(R_SM + 2), kpi.tone.soft(p));
                paint_icon(ui.painter(), rect.shrink(7.0), kpi.icon, kpi.tone.strong(p));
                ui.add(egui::Label::new(text(kpi.label, regular(FS_SM), p.text_2)).truncate());
            });
            ui.add_space(SP_1);
            let (size, height) = if kpi.small_value {
                (FS_LG, 34.0)
            } else {
                (FS_XL, 34.0)
            };
            ui.allocate_ui_with_layout(
                vec2(ui.available_width(), height),
                Layout::left_to_right(Align::Center),
                |ui| {
                    ui.add(
                        egui::Label::new(text(
                            kpi.value.clone(),
                            semibold(size),
                            kpi.value_color.unwrap_or(p.text),
                        ))
                        .truncate(),
                    );
                },
            );
            let note = if kpi.note.is_empty() { " " } else { &kpi.note };
            ui.add(egui::Label::new(text(note, regular(FS_XS), p.text_3)).truncate());
        })
        .response
}

/// Hinweiskarte: Fläche weich, links ein Rand in der kräftigen Farbe, Symbol, Titel, Text und
/// darunter Knöpfe.
pub fn notice(
    ui: &mut Ui,
    tone: Tone,
    title: &str,
    body: &str,
    actions: impl FnOnce(&mut Ui),
) -> Response {
    let p = palette(ui);
    let strong = tone.strong(p);
    let response = Frame::new()
        .fill(tone.soft(p))
        .corner_radius(CornerRadius::same(R_MD))
        .inner_margin(Margin {
            left: (SP_4 + 4.0) as i8,
            right: SP_4 as i8,
            top: SP_3 as i8,
            bottom: SP_3 as i8,
        })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_top(|ui| {
                icon(ui, tone.icon(), 20.0, strong);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = SP_1;
                    if !title.is_empty() {
                        ui.add(egui::Label::new(text(title, semibold(FS_BASE), p.text)).wrap());
                    }
                    if !body.is_empty() {
                        ui.add(egui::Label::new(text(body, regular(FS_BASE), p.text)).wrap());
                    }
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        actions(ui);
                    });
                });
            });
        })
        .response;
    let rect = response.rect;
    ui.painter().rect_filled(
        Rect::from_min_max(rect.min, pos2(rect.left() + 4.0, rect.bottom())),
        CornerRadius {
            nw: R_MD,
            sw: R_MD,
            ne: 0,
            se: 0,
        },
        strong,
    );
    response
}

/// Größe des Kennzeichens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlateSize {
    /// In Listen
    Small,
    /// Auf Karten
    Large,
}

/// Kennzeichen: weißes Schild mit 2 px Rand, blauer Streifen mit "D", Schrift 600.
pub fn plate(ui: &mut Ui, label: &str, size: PlateSize) -> Response {
    let (font, height, strip) = match size {
        PlateSize::Small => (semibold(FS_SM), 24.0, 11.0),
        PlateSize::Large => (semibold(17.0), 32.0, 14.0),
    };
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_string(), font, PLATE_TEXT);
    let width = strip + galley.size().x + 2.0 * SP_2 + 4.0;
    let (rect, response) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let radius = CornerRadius::same(5);
        painter.rect_filled(rect, radius, PLATE_BG);
        let blue = Rect::from_min_max(rect.min, pos2(rect.left() + strip + 2.0, rect.bottom()));
        painter.rect_filled(
            blue,
            CornerRadius {
                nw: 5,
                sw: 5,
                ne: 0,
                se: 0,
            },
            PLATE_BLUE,
        );
        painter.text(
            pos2(blue.center().x + 1.0, rect.bottom() - 4.0),
            egui::Align2::CENTER_BOTTOM,
            "D",
            semibold(if size == PlateSize::Large { 10.0 } else { 8.0 }),
            PLATE_BG,
        );
        painter.rect_stroke(
            rect,
            radius,
            Stroke::new(2.0, PLATE_TEXT),
            StrokeKind::Inside,
        );
        let pos = pos2(blue.right() + SP_2, rect.center().y - galley.size().y / 2.0);
        painter.galley(pos, galley, PLATE_TEXT);
    }
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, label));
    response
}

/// Schadensbalken in `width`: Füllung nach [`damage_tone`], ein Strich an der Schadensgrenze.
pub fn damage_bar(ui: &mut Ui, percent: f64, limit: f64, width: f32) -> Response {
    let p = palette(ui);
    let (rect, response) = ui.allocate_exact_size(vec2(width, 6.0), Sense::hover());
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        painter.rect_filled(rect, CornerRadius::same(3), p.border);
        let share = (percent / 100.0).clamp(0.0, 1.0) as f32;
        if share > 0.0 {
            let fill = Rect::from_min_size(rect.min, vec2((rect.width() * share).max(4.0), 6.0));
            painter.rect_filled(
                fill,
                CornerRadius::same(3),
                damage_tone(percent, limit).strong(p),
            );
        }
        if limit > 0.0 && limit < 100.0 {
            let x = rect.left() + rect.width() * (limit / 100.0) as f32;
            painter.line_segment(
                [pos2(x, rect.top() - 3.0), pos2(x, rect.bottom() + 3.0)],
                Stroke::new(1.5, p.text_3),
            );
        }
    }
    response
}

/// Initialen im Kreis.
pub fn avatar(ui: &mut Ui, name: &str, size: f32, fill: Color32, color: Color32) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    if ui.is_rect_visible(rect) {
        ui.painter().circle_filled(rect.center(), size / 2.0, fill);
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            initials(name),
            semibold(size * 0.4),
            color,
        );
    }
    response
}

/// Leerer Zustand: Symbol, ein Satz, darunter Platz für eine Aktion.
pub fn empty_state(ui: &mut Ui, symbol: Icon, sentence: &str, action: impl FnOnce(&mut Ui)) {
    let p = palette(ui);
    card(ui).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.vertical_centered(|ui| {
            ui.add_space(SP_4);
            icon(ui, symbol, 32.0, p.text_3);
            ui.add_space(SP_1);
            ui.add(egui::Label::new(text(sentence, regular(FS_BASE), p.text_2)).wrap());
            ui.add_space(SP_2);
            action(ui);
            ui.add_space(SP_2);
        });
    });
}

/// Art eines Knopfs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonKind {
    /// Eine Hauptaktion je Bereich oder Karte
    Primary,
    /// Weitere Aktionen
    Secondary,
    /// Ohne Rand, Schrift in `primary`
    Quiet,
    /// Stornieren, Abmelden und Ähnliches
    Danger,
}

/// Knopf nach dem Design-System, 32 px hoch, `r_sm`, Schrift 14 mit Gewicht 600. Deaktiviert
/// mit halber Deckkraft (über `add_enabled`), Fokus mit Ring in `primary`.
pub struct ActionButton {
    label: String,
    kind: ButtonKind,
    icon: Option<Icon>,
    full_width: bool,
}

impl ActionButton {
    pub fn new(label: impl Into<String>, kind: ButtonKind) -> Self {
        ActionButton {
            label: label.into(),
            kind,
            icon: None,
            full_width: false,
        }
    }

    pub fn primary(label: impl Into<String>) -> Self {
        Self::new(label, ButtonKind::Primary)
    }

    pub fn secondary(label: impl Into<String>) -> Self {
        Self::new(label, ButtonKind::Secondary)
    }

    pub fn quiet(label: impl Into<String>) -> Self {
        Self::new(label, ButtonKind::Quiet)
    }

    pub fn danger(label: impl Into<String>) -> Self {
        Self::new(label, ButtonKind::Danger)
    }

    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    /// Nimmt die ganze Breite ein
    pub fn full_width(mut self) -> Self {
        self.full_width = true;
        self
    }
}

impl Widget for ActionButton {
    fn ui(self, ui: &mut Ui) -> Response {
        let p = palette(ui);
        let galley = ui.painter().layout_no_wrap(
            self.label.clone(),
            semibold(FS_BASE),
            Color32::PLACEHOLDER,
        );
        let icon_width = if self.icon.is_some() { 16.0 + 6.0 } else { 0.0 };
        let pad = if self.kind == ButtonKind::Quiet {
            SP_2
        } else {
            SP_3 + 2.0
        };
        let mut width = galley.size().x + icon_width + 2.0 * pad;
        if self.full_width {
            width = width.max(ui.available_width());
        }
        let (rect, response) = ui.allocate_exact_size(vec2(width, CONTROL_HEIGHT), Sense::click());
        let enabled = ui.is_enabled();
        let label = self.label.clone();
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, &label));
        if ui.is_rect_visible(rect) {
            let hot = enabled && (response.hovered() || response.is_pointer_button_down_on());
            let (fill, stroke, fg) = match self.kind {
                ButtonKind::Primary => (
                    if hot { p.primary_hover } else { p.primary },
                    Stroke::NONE,
                    p.on_primary,
                ),
                ButtonKind::Secondary => (
                    if hot { p.surface_2 } else { p.surface },
                    Stroke::new(1.0, if hot { p.text_3 } else { p.border_strong }),
                    p.text,
                ),
                ButtonKind::Quiet => (
                    if hot {
                        p.primary_soft
                    } else {
                        Color32::TRANSPARENT
                    },
                    Stroke::NONE,
                    p.primary,
                ),
                ButtonKind::Danger => (
                    if hot {
                        p.danger.lerp_to_gamma(p.text, 0.15)
                    } else {
                        p.danger
                    },
                    Stroke::NONE,
                    p.on_primary,
                ),
            };
            let painter = ui.painter();
            painter.rect(
                rect,
                CornerRadius::same(R_SM),
                fill,
                stroke,
                StrokeKind::Inside,
            );
            if response.has_focus() {
                painter.rect_stroke(
                    rect.expand(2.0),
                    CornerRadius::same(R_SM + 2),
                    Stroke::new(2.0, p.primary),
                    StrokeKind::Outside,
                );
            }
            let content = galley.size().x + icon_width;
            let mut x = rect.center().x - content / 2.0;
            if let Some(i) = self.icon {
                let r = Rect::from_center_size(pos2(x + 8.0, rect.center().y), Vec2::splat(16.0));
                paint_icon(painter, r, i, fg);
                x += icon_width;
            }
            painter.galley(pos2(x, rect.center().y - galley.size().y / 2.0), galley, fg);
        }
        response.on_hover_cursor(egui::CursorIcon::PointingHand)
    }
}

/// Eintrag der Seitenleiste: Symbol und Text, aktiv mit `nav-active-bg` und links einer 3 px
/// breiten Leiste in `accent`.
pub fn nav_item(ui: &mut Ui, symbol: Icon, label: &str, active: bool) -> Response {
    let p = palette(ui);
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 36.0), Sense::click());
    response.widget_info(|| {
        let mut info = egui::WidgetInfo::labeled(egui::Role::Tab, true, label);
        info.selected = Some(active);
        info
    });
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let color = if active || response.hovered() {
            p.nav_active
        } else {
            p.nav_text
        };
        if active {
            painter.rect_filled(rect, CornerRadius::same(R_SM), p.nav_active_bg);
            painter.rect_filled(
                Rect::from_min_size(rect.min, vec2(3.0, rect.height())),
                CornerRadius {
                    nw: R_SM,
                    sw: R_SM,
                    ne: 0,
                    se: 0,
                },
                p.accent,
            );
        } else if response.hovered() {
            painter.rect_filled(
                rect,
                CornerRadius::same(R_SM),
                p.nav_active_bg.gamma_multiply(0.6),
            );
        }
        if response.has_focus() {
            painter.rect_stroke(
                rect,
                CornerRadius::same(R_SM),
                Stroke::new(2.0, p.accent),
                StrokeKind::Inside,
            );
        }
        let icon_rect =
            Rect::from_center_size(pos2(rect.left() + 24.0, rect.center().y), Vec2::splat(18.0));
        paint_icon(painter, icon_rect, symbol, color);
        let font = if active {
            semibold(FS_BASE)
        } else {
            regular(FS_BASE)
        };
        painter.text(
            pos2(rect.left() + 44.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            label,
            font,
            color,
        );
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Zeile Schlüssel und Wert in einer Karte, Wert rechtsbündig. Ein zu langer Wert wird gekürzt,
/// egui zeigt ihn dann ganz im Tooltip.
pub fn key_value(ui: &mut Ui, key: &str, value: &str, value_color: Option<Color32>) {
    let p = palette(ui);
    ui.allocate_ui_with_layout(
        vec2(ui.available_width(), 28.0),
        Layout::left_to_right(Align::Center),
        |ui| {
            ui.label(text(key, regular(FS_BASE), p.text_2));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add(
                    egui::Label::new(text(
                        value,
                        semibold(FS_BASE),
                        value_color.unwrap_or(p.text),
                    ))
                    .truncate(),
                );
            });
        },
    );
}

/// Feine Trennlinie in `border`.
pub fn divider(ui: &mut Ui) {
    let p = palette(ui);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::ZERO, p.border);
}

/// Die Farben, die gerade gelten, ohne eine `Ui` (für Dialoge).
pub fn palette_of(ctx: &egui::Context) -> &'static Palette {
    if ctx.global_style().visuals.dark_mode {
        &DARK
    } else {
        &LIGHT
    }
}

/// Rahmen eines Dialogs: `surface`, `r_md`, großer Schatten, viel Innenabstand.
pub fn dialog_frame(ctx: &egui::Context) -> Frame {
    let dark = ctx.global_style().visuals.dark_mode;
    let p = palette_of(ctx);
    Frame::new()
        .fill(p.surface)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(CornerRadius::same(R_MD))
        .inner_margin(Margin::same(SP_6 as i8))
        .shadow(shadow_lg(dark))
}

/// Auswählbare Zeile mit Kreis wie ein Optionsfeld, ausgewählt in `primary-soft` mit Rand in
/// `primary`. Ein Klick irgendwo auf die Zeile wählt sie.
pub fn choice_row(
    ui: &mut Ui,
    id_salt: impl egui::AsIdSalt,
    selected: bool,
    add_contents: impl FnOnce(&mut Ui),
) -> Response {
    let p = palette(ui);
    let id = ui.scope_id().with(id_salt);
    let hovered = ui.ctx().read_response(id).is_some_and(|r| r.hovered());
    let (fill, stroke) = if selected {
        (p.primary_soft, p.primary)
    } else if hovered {
        (p.surface_2, p.text_3)
    } else {
        (p.surface, p.border)
    };
    let inner = Frame::new()
        .fill(fill)
        .stroke(Stroke::new(1.0, stroke))
        .corner_radius(CornerRadius::same(R_SM))
        .inner_margin(Margin {
            left: (SP_3 + 16.0 + SP_2) as i8,
            right: SP_3 as i8,
            top: SP_2 as i8,
            bottom: SP_2 as i8,
        })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.style_mut().interaction.selectable_labels = false;
            ui.horizontal(|ui| add_contents(ui));
        });
    let rect = inner.response.rect;
    let response = ui.interact(rect, id, Sense::click());
    let center = pos2(rect.left() + SP_3 + 8.0, rect.center().y);
    let painter = ui.painter();
    painter.circle_filled(center, 8.0, p.surface);
    painter.circle_stroke(
        center,
        7.5,
        Stroke::new(1.0, if selected { p.primary } else { p.border_strong }),
    );
    if selected {
        painter.circle_filled(center, 4.0, p.primary);
    }
    if response.has_focus() {
        painter.rect_stroke(
            rect.expand(2.0),
            CornerRadius::same(R_SM + 2),
            Stroke::new(2.0, p.primary),
            StrokeKind::Outside,
        );
    }
    response.widget_info(|| {
        let mut info = egui::WidgetInfo::new(egui::Role::RadioButton);
        info.selected = Some(selected);
        info
    });
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Filter-Pille mit Anzahl, etwa "Frei 2". Aktiv in `primary-soft` mit Schrift in `primary`.
pub fn filter_pill(ui: &mut Ui, label: &str, count: usize, active: bool) -> Response {
    let p = palette(ui);
    let font = if active {
        semibold(FS_SM)
    } else {
        regular(FS_SM)
    };
    let name = ui
        .painter()
        .layout_no_wrap(label.to_string(), font, Color32::PLACEHOLDER);
    let number =
        ui.painter()
            .layout_no_wrap(count.to_string(), semibold(FS_XS), Color32::PLACEHOLDER);
    let width = SP_3 + name.size().x + SP_2 + number.size().x + 12.0 + SP_3;
    let (rect, response) = ui.allocate_exact_size(vec2(width, 30.0), Sense::click());
    response.widget_info(|| {
        let mut info =
            egui::WidgetInfo::labeled(egui::Role::Button, true, format!("{label} {count}"));
        info.selected = Some(active);
        info
    });
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let (fill, stroke, color) = if active {
            (p.primary_soft, p.primary, p.primary)
        } else if response.hovered() {
            (p.surface_2, p.text_3, p.text)
        } else {
            (p.surface, p.border_strong, p.text_2)
        };
        painter.rect(
            rect,
            CornerRadius::same(R_PILL),
            fill,
            Stroke::new(1.0, stroke),
            StrokeKind::Inside,
        );
        let y = rect.center().y;
        painter.galley(
            pos2(rect.left() + SP_3, y - name.size().y / 2.0),
            name.clone(),
            color,
        );
        let count_rect = Rect::from_min_size(
            pos2(rect.left() + SP_3 + name.size().x + SP_2, y - 9.0),
            vec2(number.size().x + 12.0, 18.0),
        );
        painter.rect_filled(
            count_rect,
            CornerRadius::same(R_PILL),
            if active { p.surface } else { p.surface_2 },
        );
        painter.galley(count_rect.center() - number.size() / 2.0, number, color);
        if response.has_focus() {
            painter.rect_stroke(
                rect.expand(2.0),
                CornerRadius::same(R_PILL),
                Stroke::new(2.0, p.primary),
                StrokeKind::Outside,
            );
        }
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Spalte fester Breite in einer Zeile, für Raster aus Karten.
pub fn column<R>(ui: &mut Ui, width: f32, add_contents: impl FnOnce(&mut Ui) -> R) -> R {
    ui.allocate_ui_with_layout(vec2(width, 0.0), Layout::top_down(Align::Min), |ui| {
        ui.set_width(width);
        add_contents(ui)
    })
    .inner
}

/// Punkt für den Anfang eines Rechtecks im 24er Raster, für Tests der Symbole.
#[cfg(test)]
fn grid_point(rect: Rect, x: f32, y: f32) -> egui::Pos2 {
    let s = rect.width().min(rect.height()) / 24.0;
    rect.center() - vec2(12.0 * s, 12.0 * s) + vec2(x * s, y * s)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PALETTES: [(&str, Palette); 2] = [("hell", LIGHT), ("dunkel", DARK)];

    #[test]
    fn tokens_match_the_design_system() {
        // Stichproben aus der Tabelle in docs/design-system.md
        assert_eq!(light::BG, Color32::from_rgb(0xf2, 0xf4, 0xf7));
        assert_eq!(dark::BG, Color32::from_rgb(0x0b, 0x12, 0x20));
        assert_eq!(light::PRIMARY, Color32::from_rgb(0x1d, 0x4e, 0xd8));
        assert_eq!(dark::SUCCESS_SOFT, Color32::from_rgb(0x12, 0x34, 0x23));
        assert_eq!(LIGHT.nav_bg, light::NAV_BG);
        assert_eq!(DARK.accent, dark::ACCENT);
    }

    #[test]
    fn text_on_surfaces_reads_well() {
        for (mode, p) in PALETTES {
            for (name, fg, bg) in [
                ("text auf bg", p.text, p.bg),
                ("text auf surface", p.text, p.surface),
                ("text auf surface-2", p.text, p.surface_2),
                ("text-2 auf bg", p.text_2, p.bg),
                ("text-2 auf surface", p.text_2, p.surface),
                ("primary auf surface", p.primary, p.surface),
                ("primary auf primary-soft", p.primary, p.primary_soft),
                ("text auf primary-soft", p.text, p.primary_soft),
                ("on-primary auf primary", p.on_primary, p.primary),
                (
                    "on-primary auf primary-hover",
                    p.on_primary,
                    p.primary_hover,
                ),
                ("on-primary auf danger", p.on_primary, p.danger),
                ("danger auf surface", p.danger, p.surface),
                ("success auf surface", p.success, p.surface),
                ("warning auf surface", p.warning, p.surface),
                ("danger auf danger-soft", p.danger, p.danger_soft),
                ("info auf info-soft", p.info, p.info_soft),
                ("text auf success-soft", p.text, p.success_soft),
                ("text auf warning-soft", p.text, p.warning_soft),
                ("text auf danger-soft", p.text, p.danger_soft),
                ("text auf info-soft", p.text, p.info_soft),
                ("nav-text auf nav-bg", p.nav_text, p.nav_bg),
                (
                    "nav-active auf nav-active-bg",
                    p.nav_active,
                    p.nav_active_bg,
                ),
                ("Kennzeichen", PLATE_TEXT, PLATE_BG),
                ("success auf success-soft", p.success, p.success_soft),
                ("warning auf warning-soft", p.warning, p.warning_soft),
                ("text-3 auf surface", p.text_3, p.surface),
            ] {
                let c = contrast(fg, bg);
                assert!(c >= 4.5, "{mode}: {name} hat nur {c:.2} zu 1");
            }
        }
    }

    #[test]
    fn hints_and_accent_keep_their_contrast() {
        // `text-3` steht fast immer auf Karten (4,5 zu 1, oben geprüft), auf dem Seitengrund
        // im hellen Modus liegt es knapp darunter. Das Orange markiert nur, es trägt keinen Text.
        for (mode, p) in PALETTES {
            for (name, fg, bg, min) in [
                ("text-3 auf bg", p.text_3, p.bg, 4.3),
                ("accent auf nav-bg", p.accent, p.nav_bg, 3.0),
            ] {
                let c = contrast(fg, bg);
                assert!(c >= min, "{mode}: {name} hat nur {c:.2} zu 1");
            }
        }
    }

    #[test]
    fn contrast_follows_wcag() {
        assert!((contrast(Color32::BLACK, Color32::WHITE) - 21.0).abs() < 1e-9);
        assert!((contrast(Color32::WHITE, Color32::WHITE) - 1.0).abs() < 1e-9);
        // Bekannter Wert: #767676 auf Weiß sind 4,54 zu 1
        let grey = Color32::from_rgb(0x76, 0x76, 0x76);
        assert!((contrast(grey, Color32::WHITE) - 4.54).abs() < 0.01);
    }

    #[test]
    fn damage_bar_color_by_percent() {
        assert_eq!(damage_tone(0.0, 25.0), Tone::Success);
        assert_eq!(damage_tone(9.9, 25.0), Tone::Success);
        assert_eq!(damage_tone(10.0, 25.0), Tone::Warning);
        assert_eq!(damage_tone(25.0, 25.0), Tone::Warning);
        assert_eq!(damage_tone(25.1, 25.0), Tone::Danger);
        assert_eq!(damage_tone(100.0, 25.0), Tone::Danger);
        // Liegt die Grenze unter 10 %, ist alles darüber rot
        assert_eq!(damage_tone(7.0, 5.0), Tone::Danger);
        assert_eq!(damage_tone(4.0, 5.0), Tone::Success);
    }

    #[test]
    fn card_fits_both_buttons_of_an_own_lease() {
        // Zwei Knöpfe der eigenen Leihe brauchen mit Innenabstand rund 366 px
        const { assert!(CARD_MIN_WIDTH >= 370.0) };
        // Bei der kleinsten Breite jeder Spalte bleibt die Karte mindestens so breit
        for full in [400.0, 700.0, 736.0, 960.0, 1121.0, 1400.0] {
            let (cols, width) = grid_columns(full, CARD_MIN_WIDTH, SP_4);
            assert!(
                cols == 1 || width >= CARD_MIN_WIDTH,
                "{full}: {cols} Spalten zu {width}"
            );
        }
    }

    #[test]
    fn model_names_read_like_the_dashboard() {
        assert_eq!(model_name("scania.s_2016"), "Scania S 2016");
        assert_eq!(model_name("volvo.fh_2024"), "Volvo FH 2024");
        assert_eq!(model_name("mercedes.actros"), "Mercedes Actros");
        assert_eq!(model_name("daf.xf_euro6"), "DAF XF Euro 6");
        assert_eq!(model_name("volvo.fh16_2012"), "Volvo FH16 2012");
        assert_eq!(model_name("mercedes.actros2014"), "Mercedes Actros 2014");
        assert_eq!(model_name("man.tgx_euro6"), "MAN TGX Euro 6");
        // Kein Wort ganz groß, nur weil es kurz ist
        assert_eq!(model_name("mercedes.new_actros"), "Mercedes New Actros");
        assert_eq!(model_name("daf.2021"), "DAF 2021");
        assert_eq!(model_name("renault.t"), "Renault T");
        assert_eq!(model_name(""), "");
        assert_eq!(model_name("._"), "");
    }

    #[test]
    fn numbers_get_german_separators() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1000), "1.000");
        assert_eq!(thousands(120_500), "120.500");
        assert_eq!(thousands(1_234_567), "1.234.567");
        assert_eq!(thousands(-3870), "-3.870");
        assert_eq!(thousands(i64::MIN), "-9.223.372.036.854.775.808");
        assert_eq!(euro(3870), "3.870 €");
        assert_eq!(euro(-50), "-50 €");
        assert_eq!(km(88_000), "88.000 km");
    }

    #[test]
    fn initials_for_the_circle() {
        assert_eq!(initials("Anna Berg"), "AB");
        assert_eq!(initials("Chefin"), "CH");
        assert_eq!(initials("anna maria berg"), "AB");
        assert_eq!(initials("Ö"), "Ö");
        assert_eq!(initials("  "), "?");
    }

    #[test]
    fn cards_fill_the_row() {
        // 1000 px mit 320er Karten und 16 px Lücke: drei passen (992 px), vier nicht
        let (n, w) = grid_columns(1000.0, 320.0, 16.0);
        assert_eq!(n, 3);
        assert!((w - 322.666).abs() < 0.01);
        assert_eq!(grid_columns(992.0, 320.0, 16.0).0, 3);
        assert_eq!(grid_columns(991.0, 320.0, 16.0).0, 2);
        // Zu schmal: trotzdem eine Spalte
        let (n, w) = grid_columns(200.0, 320.0, 16.0);
        assert_eq!((n, w), (1, 200.0));
    }

    #[test]
    fn theme_from_the_environment() {
        assert_eq!(theme_preference(None), ThemePreference::System);
        assert_eq!(theme_preference(Some("dunkel")), ThemePreference::Dark);
        assert_eq!(theme_preference(Some(" Hell ")), ThemePreference::Light);
        assert_eq!(theme_preference(Some("dark")), ThemePreference::Dark);
        assert_eq!(theme_preference(Some("bunt")), ThemePreference::System);
    }

    #[test]
    fn semibold_family_always_exists() {
        for fonts in [
            font_definitions(None, None),
            font_definitions(Some(Vec::new()), None),
        ] {
            let family = FontFamily::Name(SEMIBOLD.into());
            assert!(!fonts.families[&family].is_empty());
            assert!(!fonts.families[&FontFamily::Proportional].is_empty());
        }
        // Eine lesbare Schrift (hier eine von egui selbst) kommt nach vorn
        let good = builtin_font();
        let fonts = font_definitions(Some(good.clone()), Some(good));
        assert_eq!(
            fonts.families[&FontFamily::Name(SEMIBOLD.into())][0],
            "system-semibold"
        );
        assert_eq!(fonts.families[&FontFamily::Proportional][0], "system");
    }

    /// Bytes einer eingebauten Schrift von egui, als Beispiel für eine gültige Datei.
    fn builtin_font() -> Vec<u8> {
        let fonts = FontDefinitions::default();
        let name = &fonts.families[&FontFamily::Proportional][0];
        (*fonts.font_data[name].font).as_ref().to_vec()
    }

    #[test]
    fn broken_font_falls_back_instead_of_crashing() {
        assert!(font_works(&builtin_font()));
        // Leer oder Müll: nicht nehmen. (Eine abgeschnittene Datei mit gültigem Kopf lässt egui
        // durchgehen, ohne abzustürzen, dann zeichnet es mit den eingebauten Schriften weiter.)
        for bad in [Vec::new(), vec![1, 2, 3], vec![0; 4096]] {
            assert!(!font_works(&bad), "{} Bytes", bad.len());
            let fonts = font_definitions(Some(bad.clone()), Some(bad));
            assert!(!fonts.font_data.contains_key("system"));
            assert!(!fonts.font_data.contains_key("system-semibold"));
            // Mit den eingebauten Schriften lässt sich Text auslegen, ohne Panic
            let ctx = egui::Context::default();
            ctx.set_fonts(fonts);
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                ui.label(text("Probe", semibold(FS_BASE), Color32::BLACK));
            });
            out.textures_delta.clear();
        }
    }

    #[test]
    fn style_uses_the_tokens() {
        let s = style(&LIGHT, false);
        assert_eq!(s.visuals.panel_fill, light::BG);
        assert_eq!(s.visuals.window_fill, light::SURFACE);
        assert_eq!(s.visuals.selection.stroke.color, light::PRIMARY);
        assert_eq!(s.spacing.interact_size.y, CONTROL_HEIGHT);
        assert_eq!(
            s.visuals.widgets.inactive.corner_radius,
            CornerRadius::same(R_SM)
        );
        let d = style(&DARK, true);
        assert!(d.visuals.dark_mode);
        assert_eq!(d.visuals.panel_fill, dark::BG);
        // Abstände im 4er Raster
        for v in [
            s.spacing.item_spacing.x,
            s.spacing.item_spacing.y,
            s.spacing.indent,
            SP_1,
            SP_3,
            SP_5,
            SP_6,
            SP_8,
            SP_10,
        ] {
            assert_eq!(v % 4.0, 0.0);
        }
    }

    #[test]
    fn icons_stay_in_their_box() {
        let rect = Rect::from_min_size(pos2(10.0, 20.0), Vec2::splat(48.0));
        assert_eq!(grid_point(rect, 0.0, 0.0), rect.min);
        assert_eq!(grid_point(rect, 24.0, 24.0), rect.max);
        for (x, y) in arc((12.0, 12.0), 9.5, 0.0, 360.0) {
            assert!((0.0..=24.0).contains(&x) && (0.0..=24.0).contains(&y));
        }
    }
}
