use knf::{InterpolationOptions, MergeOptions, merge_layers};
use knf_convert::{Layout, OutputFormat, Policy, load_layers};

fn main() -> anyhow::Result<()> {
    let layers = load_layers(&["base.toml", "prod.toml"], None, None)?;
    let parse_inline = |text| layers.format.parse_inline(text);
    let interpolation = Some(InterpolationOptions::new(&parse_inline));
    let merged = merge_layers(layers.values, &MergeOptions::default(), interpolation)?;
    let json = OutputFormat::Json.emit(&merged, Policy::Strict, Layout::Pretty)?;
    print!("{}", json.value);
    Ok(())
}
