# Anamorphic Solar Camera Effect

## Provenance

The base RenderingDevice compositor is adapted from
https://github.com/ARez2/compositor-effect-lens-effects
at commit `0ce061271f672f05fbb9aed08a61e019a875c1ad`, under MIT (see LICENSE).
The base derives from Pink Arcana's
https://github.com/pink-arcana/godot-distance-field-outlines .

Dawn's lens script and compute shader use a shared stellar-source model and a
normalized lens PSF. Godray remains a separate depth-occlusion approximation;
the upstream scene-buffer layout and full flare shader are not used unchanged.

## Integration

`WorldPresentation` supplies the source snapshot and attaches the compositor
where a RenderingDevice is available. The camera preset is
`client/resources/solar_anamorphic_lens.tres`. Without the compositor the
physical sun still renders.

Pipeline, ownership, settings and approximation limits:
[Celestial Rendering](../../../docs/architecture/celestial-rendering.md).

Review commands and GPU regression procedures:
[Godot Client Testing](../../../docs/process/godot-client-testing.md#fixed-lighting-review).

Architectural rationale:
[ADR-0053](../../../docs/adr/ADR-0053-celestial-presentation-boundary.md).
