# Mathcad and SMath test corpus (spike)

Collected 2026-10-19 for the Rezzer calculation-worksheet research
(`docs/research/2026-10-19-mathcad.md`). Every file comes from a public GitHub repository
whose repository license allows redistribution. No repository states a separate license for
its sample files, so the repository license is taken to cover them. Source links are pinned
to the commit that was current on the download date.

These files are untrusted data. Never open them in Mathcad or SMath Studio with automatic
calculation on (worksheets can call scripted controls, DLL functions and file read/write
functions such as WRITEPRN), never execute anything inside them, keep all scripts outside
this folder and pass paths as arguments.

Provenance caveat: the mcad2py files have 1980-01-01 zip timestamps and empty
`dc:creator`, and the repository has tools named `strip_mcdx_metadata.py` and
`recalc_mcdx.py`. They were probably re-zipped or edited by a tool after Prime saved them,
so they are not guaranteed to be untouched Prime output. `mpt_JSON.mcdx` and
`netefi-mp6.mcdx` carry real Prime timestamps.

Known gaps: no binary `.mcd` file (none found under a usable license), no `.xmcdz`, no
Mathcad 12, 13 or 15 file that says so in its `generator` element (the NetEFI "mc15" file
says 14.1), no Prime 1 or 2 file, no worksheet with a password protected or locked area, no
SMath Studio 1.x file, no SMath plot region.

## Mathcad Prime (.mcdx)

| File | Prime version (app.xml) | License | Source | sha256 (first 16) | What it shows |
|---|---|---|---|---|---|
| `mcdx/mcad2py_RC_col.mcdx` | 11.0.1.0 | MIT (ChristianCruger/mcad2py) | [source](https://github.com/ChristianCruger/mcad2py/blob/778f42003f92b900a48d5aeb7cd5d2e3fafe8acc/references/RC_col.mcdx) | `6ef5fd2b363d674f` | Real engineering sheet: RC column biaxial bending, units, programs, matrices, xy plots with cached trace data, 96 text regions |
| `mcdx/mcad2py_Constants.mcdx` | 11.0.1.0 | MIT (mcad2py) | [source](https://github.com/ChristianCruger/mcad2py/blob/778f42003f92b900a48d5aeb7cd5d2e3fafe8acc/references/Constants.mcdx) | `07c7aa7ec348281e` | Built-in constants (`labels="CONSTANT"`) |
| `mcdx/mcad2py_solve_as_function.mcdx` | 11.0.1.0 | MIT (mcad2py) | [source](https://github.com/ChristianCruger/mcad2py/blob/778f42003f92b900a48d5aeb7cd5d2e3fafe8acc/references/solve_as_function.mcdx) | `b9976b64fe342a2d` | Solve block with `solve-block-category` guess-value, constraint, solver |
| `mcdx/mcad2py_mixed_plot_traces.mcdx` | 11.0.1.0 | MIT (mcad2py) | [source](https://github.com/ChristianCruger/mcad2py/blob/778f42003f92b900a48d5aeb7cd5d2e3fafe8acc/references/mixed_plot_traces.mcdx) | `2ad5a5c903ee2ffc` | XY plot with several traces |
| `mcdx/mcad2py_header_footer.mcdx` | 12.0.0.1 | MIT (mcad2py) | [source](https://github.com/ChristianCruger/mcad2py/blob/778f42003f92b900a48d5aeb7cd5d2e3fafe8acc/references/header_footer.mcdx) | `238c35a2ba2ad3e5` | Header and footer parts, page number fields, `Pending` results, engine error with message |
| `mcdx/mcad2py_collapsable-area.mcdx` | 11.0.1.0 | MIT (mcad2py) | [source](https://github.com/ChristianCruger/mcad2py/blob/778f42003f92b900a48d5aeb7cd5d2e3fafe8acc/references/collapsable-area.mcdx) | `c1355754536a0b02` | Collapsible `<Area>` with nested regions (not protected) |
| `mcdx/mcad2py_matrices.mcdx` | 11.0.1.0 | MIT (mcad2py) | [source](https://github.com/ChristianCruger/mcad2py/blob/778f42003f92b900a48d5aeb7cd5d2e3fafe8acc/references/matrices.mcdx) | `6b0c9f4d9d415030` | Matrix literals and matrix results |
| `mcdx/mpt_JSON.mcdx` | 4.0.0.0 | MIT (jakemckown/mathcad-prime-tools) | [source](https://github.com/jakemckown/mathcad-prime-tools/blob/d12fa8ac2fe91d5f46ee51319bf8fa925e16bfc9/json/JSON.mcdx) | `be85f89949d8ba6a` | Prime 4 file, user functions and programs, no text parts, author in core.xml |
| `mcdx/netefi-mp6.mcdx` | 6.0.0.0 | MIT (ViacheslavMezentsev/NetEFI) | [source](https://github.com/ViacheslavMezentsev/NetEFI/blob/df8168ea75457cd8e1426f4e5bde902a72f7abfc/samples/worksheets/netefi-mp6.mcdx) | `951e05bbd523491b` | Prime 6 file, custom DLL functions, undefined-variable engine errors, `cache-worksheet="false"` |
| `mcdx/coolprop_CoolPropFluidProperties.mcdx` | 3.0.0.0 | MIT (CoolProp/CoolProp-museum) | [source](https://github.com/CoolProp/CoolProp-museum/blob/b309bf76dd7d2355752714a8dd8643271ae5264a/wrappers/MathCAD/Prime/CoolPropFluidProperties.mcdx) | `4aa37f16485ddf5a` | Prime 3 file (oldest seen), fluid property calls, `Pending` results |

## Mathcad 12-15 (.xmcd)

| File | Generator | License | Source | sha256 (first 16) | What it shows |
|---|---|---|---|---|---|
| `xmcd/csamp_Hand_Calculations.xmcd` | Mathcad Professional 14.1 | MIT (junior-jl/cs-amp-sky130) | [source](https://github.com/junior-jl/cs-amp-sky130/blob/cb02961a8a3ceb6637031b300032e7d1268d8d2f/Hand_Calculations.xmcd) | `a0aa6dbec40b0bd7` | Amplifier hand calculation, units, inline results with unit override, metadata with author |
| `xmcd/coolprop_CoolPropFluidProperties.xmcd` | Mathcad Professional 14.0 | MIT (CoolProp/CoolProp-museum) | [source](https://github.com/CoolProp/CoolProp-museum/blob/b309bf76dd7d2355752714a8dd8643271ae5264a/wrappers/MathCAD/CoolPropFluidProperties.xmcd) | `074b7b1403452fcb` | Same sheet as the Prime 3 file above (old and new format of one calculation), 2D plots as gzip binary items, collapsed area with lock ids |
| `xmcd/netefi-mc15.xmcd` | Mathcad Professional 14.1 | MIT (NetEFI) | [source](https://github.com/ViacheslavMezentsev/NetEFI/blob/df8168ea75457cd8e1426f4e5bde902a72f7abfc/samples/worksheets/netefi-mc15.xmcd) | `ed624be18136e975` | Named "mc15" by its author; same content as `netefi-mp6.mcdx` |
| `xmcd/vpr_example_mathcad.xmcd` | Mathcad Professional 14.1 | CC0-1.0 (xarll/vpr) | [source](https://github.com/xarll/vpr/blob/c6f1caed7e440908956201831f50896aef3b243c/items/vychmat/lab/lab1/example_mathcad.xmcd) | `bf06beab7a3c67a0` | Numerical methods lab (Russian text), plots, symbolic evaluation |
| `xmcd/heatgrad_dimensional_calculations.xmcd` | Mathcad Professional 14.0 | MIT (Alexis-D-ff/Heat-gradient-modelling) | [source](https://github.com/Alexis-D-ff/Heat-gradient-modelling/blob/bb9d8887cfc703dfcbfb2d3f21df45532d86495b/sources/dimensional_calculations.xmcd) | `6f79180fd54949f6` | Large sheet (430 KB), provenance elements, symbolic evaluation, 8 plots, mixed `is-protected` values |

## SMath Studio (.sm)

| File | SMath version | License | Source | sha256 (first 16) | What it shows |
|---|---|---|---|---|---|
| `sm/calculix_Beam.sm` | 0.98.6398 | MIT (calculix/CalculiX-Examples) | [source](https://github.com/calculix/CalculiX-Examples/blob/316273e9105e44ce7e3ee05059dac1bc3f256a69/Elements/Beam.sm) | `24dafaaa166a67a1` | Beam reference solution, units, cached numeric results with display unit, embedded PNG picture, namespaced root |
| `sm/calculix_Beam_Theory.sm` | 0.97.5737 | MIT (CalculiX-Examples) | [source](https://github.com/calculix/CalculiX-Examples/blob/316273e9105e44ce7e3ee05059dac1bc3f256a69/Elements/Beam%20Theory.sm) | `bf8af8b3632bb98b` | Older SMath version, root without namespace, German text |
| `sm/calculix_Stressconc1.sm` | 0.98.6179 | MIT (CalculiX-Examples) | [source](https://github.com/calculix/CalculiX-Examples/blob/316273e9105e44ce7e3ee05059dac1bc3f256a69/Linear/StressConc1/Stressconc1.sm) | `3436c5dd74dd66e5` | Stress concentration sheet |
