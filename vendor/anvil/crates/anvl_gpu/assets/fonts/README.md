# Bundled fonts

The engine compiles these fonts into the binary with `include_bytes!` (see `src/text.rs`). The app never falls back to a system font.

| File | Family | Weight | Used for |
|---|---|---|---|
| `Inter-Regular.ttf` | Inter | 400 | Labels, body UI text, numbers |
| `Inter-Medium.ttf` | Inter | 500 | Headings, buttons, tabs, badges |
| `Inter-SemiBold.ttf` | Inter | 600 | Display sizes only: screen titles, headline numbers |
| `Inter-Italic.ttf` | Inter | 400 italic | Emphasis, quotes (also stands in for body italic) |
| `Inter-MediumItalic.ttf` | Inter | 500 italic | Emphasis in headings and labels |
| `NotoSans-Regular.ttf` | Noto Sans | 400 | Paragraphs (body role) |
| `NotoSans-Medium.ttf` | Noto Sans | 500 | Strong text inside paragraphs |
| `FiraMono-Medium.ttf` | Fira Mono | 500 | Code and aligned figures (mono role). Its only weight |

## Provenance

Static instances of the OFL-licensed variable fonts `Inter` (wght axis) and `Noto Sans` (wght and wdth axes), taken from the `shadcn-egui-font-inter` and `shadcn-egui-font-noto-sans` crates (v0.6.0). They were produced with fontTools:

```bash
python -m fontTools.varLib.instancer inter.ttf wght=400 opsz=14 -o Inter-Regular.ttf --update-name-table
python -m fontTools.varLib.instancer inter.ttf wght=500 opsz=14 -o Inter-Medium.ttf --update-name-table
python -m fontTools.varLib.instancer noto-sans.ttf wght=400 wdth=100 -o NotoSans-Regular.ttf --update-name-table
```

The name tables were then set so both Inter weights share the family `Inter` and differ by weight class (400 and 500). That is what lets `Family::Name("Inter")` resolve to the right face.

ADL-V3 added five faces, made the same way from fonts already in the cargo registry (no download): the SemiBold and Noto Medium instances from the same variable fonts, the italics from `InterVariable-Italic.ttf` (cosmic-text 0.19, Inter 4.001, OFL), and Fira Mono Medium copied as-is from cosmic-text 0.19 (OFL):

```bash
# fontTools instancer, then name tables set so each face joins its family by weight and slant
inter.ttf                 wght=600 opsz=14  -> Inter-SemiBold.ttf      (Inter, SemiBold, 600)
InterVariable-Italic.ttf  wght=400 opsz=14  -> Inter-Italic.ttf        (Inter, Italic, 400, italic bit)
InterVariable-Italic.ttf  wght=500 opsz=14  -> Inter-MediumItalic.ttf  (Inter, Medium Italic, 500, italic bit)
noto-sans.ttf             wght=500 wdth=100 -> NotoSans-Medium.ttf     (Noto Sans, Medium, 500)
```

A role only asks for weights it has a face for (`FontRole::weight_for`), because cosmic-text falls back to another family on a weight miss.

Licenses are in `LICENSE-Inter.txt`, `LICENSE-NotoSans.txt` and `LICENSE-FiraMono.txt` (SIL Open Font License 1.1).
