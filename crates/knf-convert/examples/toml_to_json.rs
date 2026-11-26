use knf::{InterpolationOptions, MergeOptions, merge_configs};
use knf_convert::{Emit, Layout, LoadOptions, Policy, load_configs};

fn main() -> anyhow::Result<()> {
    let configs = load_configs(&["base.toml", "prod.toml"], LoadOptions::default())?;
    let parse_inline = |text| configs.format.parse_inline(text);
    let interpolation = Some(InterpolationOptions::new(&parse_inline));
    let merged = merge_configs(configs.values, &MergeOptions::default(), interpolation)?;
    let json = Emit::Json(Layout::Pretty).emit(&merged, Policy::Strict)?;
    print!("{}", json.value);
    Ok(())
}
