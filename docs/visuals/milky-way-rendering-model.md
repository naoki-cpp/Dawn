# Milky Way Rendering Model

The background is a line-of-sight radiative-transfer approximation, not a tuned
nebula texture. It separates stellar emissivity, foreground dust, and the final
mapping into Godot's linear HDR range.

## Coordinates and Stellar Emissivity

Positions use Galactic Cartesian coordinates in kpc. The Galactic centre is the
origin, the disk plane is XZ, and Y is height above the plane. The Sun is at
`(-8.2, +0.023, 0)` kpc.

The smooth disk is the sum of locally normalized thin and thick exponential
components:

```text
rho_i(R,z) = exp((R0 - R) / Rd_i) * exp(-|z| / hz_i)
j_V(R,z)   = j_V,local * [0.88 rho_thin + 0.12 rho_thick + rho_bulge]
```

The thin/thick radial scale lengths (2.12/3.05 kpc) and scale heights
(0.205/0.640 kpc) are one observation-based Milky Way mass model, not universal
constants. The bulge is a triaxial Gaussian proxy, rotated by 27 degrees. Its
normalization is derived from an adopted bulge-to-total stellar mass fraction
of 0.15 and the integrated disk profile; it is not a hand-set brightness boost.
The first-order model assumes a common mass-to-light ratio for disk and bulge.

Local V-band volume emissivity is `0.071 Lsun/pc^3`. A 5000 K Planck spectrum is
used as a compact luminosity-weighted colour approximation; this is not full
stellar-population synthesis.

## Dust and Radiative Transfer

The mean diffuse dust density is a normalized double exponential:

```text
rho_d(R,z) = exp((R0 - R) / 3.192 kpc)
             * exp((|z_sun| - |z|) / 0.103 kpc)
```

The dimensions come from a LAMOST-based dust-disk model. The initial local
diffuse normalization is `0.7 mag/kpc` in V; real sightlines vary substantially.

For diffuse sky radiance, that mean density is multiplied by a mean-normalized
lognormal realization of unresolved, supersonic dust-cloud structure. The
variance derived from `b = 0.4`, `M = 5` is split between 3D cloud structure and
a direction-coherent projected column. The latter stops broad dust clouds from
vanishing when the 3D field is integrated along a long sightline. Both use
three decorrelated scales with weights 0.55/0.30/0.15; the projected scales are
0.10/0.035/0.012 radians. The supplied 2D reference uses 11x17, 38x61, and
115x175 smooth-noise grids. The angular scales approximate those broad-to-fine
screen correlations for the game's view; they are visual priors, not measured
Galactic cloud sizes or a power spectrum. Their combined log-density variance
remains the value from the compressible-turbulence approximation, rather than
adding a second full variance on top. Dust attenuates light only in front of
each emitting volume element.

The disk's unresolved stellar-light variation is applied after the 3D
line-of-sight integral, with broad and middle correlated angular fields. This
prevents the variation from disappearing when many distant volume samples are
summed, while leaving the integrated bulge component and 3D dust transfer
unchanged. Its 0.24/0.08-radian coherence lengths and 0.20/0.11 amplitudes are
rendering priors adapted from the supplied 2D procedural reference; they are
not measured Galactic power-spectrum values and do not encode star positions.
The star sprites use the smooth mean dust profile so the 32,000-point catalogue
does not require per-star volumetric noise integration.

For each point along a ray, optical depth accumulates only from foreground
dust before the emitting volume element. The sky intensity follows:

```text
I_lambda(direction) = integral[j_lambda(s) * exp(-tau_lambda(s)) ds] / (4 pi)
A_lambda = (A_lambda / A_V) * A_V
T_lambda = 10^(-0.4 * A_lambda)
```

RGB samples at 650/550/450 nm use approximate CCM89 `R_V = 3.1` ratios
`A_lambda/A_V = 0.84/1.00/1.32`. The path integral uses non-uniform quadratic
steps so the nearby, thin dust layer is resolved without spending uniform
high-resolution samples across the whole Galaxy.

`display_radiance_scale` converts physical relative radiance into Godot's
renderer units. It is display adaptation, not a change to the Galaxy's geometry,
stellar emissivity, or extinction.

## Emission and Dust Structure

The shader integrates unresolved stellar continuum through a foreground dust
column. It does not add H-alpha, H II regions, or other gas emission: without
ionizing-source locations, gas state, and observed emission measures, such
spots would be invented light rather than a reusable astrophysical model.
Correlated structure belongs to the dust-density field and therefore appears
as wavelength-dependent attenuation of the modeled background. This follows
the reference's useful separation between a smooth Galactic band, multi-scale
cloud structure, and foreground transmission while retaining Dawn's 3D
geometry and line-of-sight integration instead of its hand-authored 2D band.

## Procedural Point Stars

The point-star layer remains a separate, deterministic sample: most sources are
drawn from the Galactic disk and bulge, with a local isotropic foreground.
Generated point brightness uses a capped Pareto-like tail (shape 1.8, scale
0.12, baseline 0.035) with a renderer gain of 3.8, adapted from the supplied
reference. The synthetic flux cap stays below Sirius on the shared display
scale. This is a visual population prior, not an observed luminosity function.
Named catalogue stars retain their visual-magnitude flux conversion. Point
sprites integrate the projected dust-column prior only to each star's
distance, so foreground stars are not dimmed by dust behind them and their
dark lanes align with the diffuse background. The point-spread profile also
uses a common 0.72-pixel Gaussian core and a 3-pixel halo whose peak is 2% of
the core. This follows the reference's shared Gaussian blur instead of changing
individual star sizes; it is a display-space approximation, not atmospheric
scintillation or a claim about each star's physical disc.

## Limits

- No small-scale 3D dust map, molecular-cloud catalogue, or Gaia pixels are
  shipped or required. The correlated field is a statistical prior, not a
  substitute for observed clouds or filaments.
- No H II emission, supernova bubbles, magnetic field, or ionizing-source
  transfer is simulated.
- The smooth stellar disk, Gaussian bulge, and single-temperature spectrum are
  deliberate first-order approximations; age, metallicity, and population
  synthesis remain outside this model.
- Procedural point sprites remain a render sample. The clumpy dust field is
  applied to diffuse sky radiance; stars use its smooth mean extinction model.

## References

- Barros et al. (2016), *Models for the 3D axisymmetric gravitational potential
  of the Milky Way galaxy*, Table 1:
  <https://www.aanda.org/articles/aa/pdf/2016/09/aa27535-15.pdf>
- Licquia & Newman (2015), Milky Way stellar mass and bulge-to-total estimate:
  <https://doi.org/10.1088/0004-637X/806/1/96>
- Hayashi (1982), local Galactic-disk surface brightness and volume emissivity:
  <https://academic.oup.com/pasj/article/34/1/89/8122339>
- Li et al. (2018), LAMOST-based Galactic dust disk:
  <https://arxiv.org/abs/1803.10540>
- Schultheis et al. (2014), diffuse extinction normalization used as an initial
  approximation:
  <https://www.aanda.org/articles/aa/pdf/2014/06/aa22788-13.pdf>
- Federrath, Klessen & Schmidt (2008), log-density variance in compressible
  turbulence:
  <https://arxiv.org/abs/0808.0605>
- Hou & Han (2014), spiral-arm tracer distributions and model uncertainty:
  <https://arxiv.org/abs/1407.7331>
- Cardelli, Clayton & Mathis (1989), extinction law; calibration curves hosted
  by STScI:
  <https://www.stsci.edu/hst/instrumentation/reference-data-for-calibration-and-tools/astronomical-catalogs/interstellar-extinction-curves>
