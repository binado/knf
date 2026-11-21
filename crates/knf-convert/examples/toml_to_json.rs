use knf::{InterpolationOptions, MergeOptions, ProcessEnv, merge_with_interpolation};
use knf_convert::{Layout, OutputFormat, Policy, load_layers};

fn main() -> anyhow::Result<()> {
    let layers = load_layers(&["base.toml", "prod.toml"])?;
    let options = InterpolationOptions {
        context: None,
        env: &ProcessEnv,
        parse_inline: &|text| layers.format.parse_inline(text),
    };
    let merged = merge_with_interpolation(layers.values, &MergeOptions::default(), &options)?;
    let json = OutputFormat::Json.emit(&merged, Policy::Strict, Layout::Pretty)?;
    print!("{}", json.value);
    Ok(())
}
