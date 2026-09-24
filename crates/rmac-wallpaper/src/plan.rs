use crate::{parse_source, Source, SourceErrorKind, FALLBACK_BUILT_IN};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Issue {
    pub output: rmac_compositor::OutputId,
    pub kind: SourceErrorKind,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Surface {
    pub output: rmac_compositor::OutputId,
    pub logical_size: rmac_compositor::LogicalSize,
    pub scale: f64,
    pub fit: rmac_shell_settings::WallpaperFit,
    pub source: Source,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Plan {
    pub surfaces: Vec<Surface>,
    pub issues: Vec<Issue>,
}

/// Build one background surface per enabled, geometrically valid output.
/// Invalid per-output sources fall back locally and never blank other outputs.
pub fn plan(
    settings: &rmac_shell_settings::WallpaperSettings,
    outputs: &[rmac_compositor::Output],
) -> Plan {
    let mut enabled: Vec<_> = outputs
        .iter()
        .filter_map(|output| {
            let logical = output.logical.as_ref()?;
            (output.current_mode.is_some()
                && logical.size.is_valid()
                && logical.size.width > 0.0
                && logical.size.height > 0.0
                && logical.scale.is_finite()
                && logical.scale > 0.0)
                .then_some((output, logical))
        })
        .collect();
    enabled.sort_by(|left, right| left.0.id.cmp(&right.0.id));

    let mut plan = Plan::default();
    for (output, logical) in enabled {
        let selection = settings
            .per_output
            .get(&output.id.0)
            .unwrap_or(&settings.default);
        let source = match parse_source(selection.source.as_deref()) {
            Ok(source) => source,
            Err(kind) => {
                plan.issues.push(Issue {
                    output: output.id.clone(),
                    kind,
                });
                Source::BuiltIn(FALLBACK_BUILT_IN)
            }
        };
        plan.surfaces.push(Surface {
            output: output.id.clone(),
            logical_size: logical.size,
            scale: logical.scale,
            fit: selection.fit,
            source,
        });
    }
    plan
}
