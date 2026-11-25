# Managed color workflow

## Version 1 numerical reference

The timeline working space is linear ACEScg/AP1 with its D60 white point. Its
RGB components are straight alpha and remain signed until the final output
transform. Diffuse reference white is 203 cd/m² in HDR conversions. Legacy SDR
projects retain their existing sRGB presentation and Rec.709 export.

Authoritative definitions and test anchors:

- [ACEScg specification](https://docs.acescentral.com/encodings/acescg/) defines AP1 and the encoding white point.
- [ITU-R BT.2100](https://www.itu.int/rec/R-REC-BT.2100) defines the PQ and HLG transfer equations. The PQ checks cover 100, 203 and 1,000 cd/m². HLG checks cover black, its knee and peak scene signal.
- [ITU-R BT.2408](https://www.itu.int/pub/R-REP-BT.2408) defines the production reference white convention.
- [Sony S-Log3 technical summary](https://www.sony.jp/ls-camera/knowledge/pdf/TechnicalSummary_for_S-Gamut3Cine_S-Gamut3_S-Log3_V1_00.pdf) supplies code values 95 for black and 420 for 18% grey. The S-Log3 transfer is shared by the two Sony gamut presets. Gamut conversion belongs to the corresponding input transform.

Input interpretation selects the explicit user override first, then decoded
frame metadata, and finally container metadata when it can fill a missing
field. An unknown HDR transfer requires clarification of the source format.
Pixel storage depth stays independent of color interpretation.

HDR10 export acceptance requires independent decoding of the rendered file,
including Main10 depth, BT.2020/PQ signalling, the actual mastering-display
declaration and timestamps. Content light levels need their own measurement
or an explicit declaration.

## BT.2020 input and SDR viewing

OpenQuartz Studio imports seekable HEVC Main10 files with explicit BT.2020
nonconstant matrix and PQ or HLG metadata. The Library displays the hardware
detected transfer and bit depth. Source preview and the SDR timeline use the
VideoToolbox P010 planes on the GPU. The SDR view maps luminance using a smooth
shoulder after conversion to linear Rec.709. Its HLG reference display uses
1,000 cd/m² and system gamma 1.2.

The native E2E case generates tagged 10-bit neutral clips, checks the monitor
against the expected SDR code values for each transfer, and checks that the
asset interpretation survives saving and reopening.

## HDR10 export

In the MP4 export dialog select **HDR10 PQ · BT.2020 / 1000 nits**. Visible
video tracks composite in the linear timeline space. The preset declares a
BT.2020/D65 1000 cd/m² mastering
target and 0.005 cd/m² minimum. Use the preset for material mastered to that
target. The encoder writes an MP4 mastering display box. The target omits
MaxCLL and MaxFALL.

The export worker reads original VideoToolbox planes into AP1 RGBA16Float.
Composition and Color Corrector work on that linear signal. The final shader
converts to BT.2020 PQ and writes limited-range 10-bit P010 directly into
IOSurface planes for VideoToolbox HEVC Main10. SDR H.264 stays the default.
A queued export captures the selected profile. Nested sequences and legacy
custom shader effects are rejected for this profile.

The E2E case verifies Main10 and its BT.2020/PQ tags using an independent
decoder, calculates the five-frame output range, checks the MP4 mastering
box, and compares both decoded 10-bit luminance patches with the original
PQ sample values. A two-track render checks that 50% opacity over a
one-stop darker background preserves the expected linear-light blend in the
decoded HDR10 signal.

## HLG export

Select **HLG · BT.2020** to write a Main10 MP4 with the BT.2100 HLG transfer.
The GPU converts the AP1 program to BT.2020 and applies the inverse display
transform for the 1,000 cd/m² reference and gamma 1.2 before the HLG OETF.
The file signals HLG in its HEVC and container color tags. The PQ mastering
display box belongs to the HDR10 option. Both profiles retain the selected
range when queued. The native E2E suite decodes HLG output independently
and checks the transfer, exact frame count, zero-based PTS and two 10-bit
luminance patches.

## Managed sequence monitoring

Enable **Managed color · ACEScg** in Sequence settings to grade and composite
in AP1 before the monitor's SDR display transform. The setting belongs to the
selected sequence, is saved in the project, and is undoable. SDR export of a
managed sequence uses the same view transform. HDR10 export sends the
linear program to the PQ output transform.

Existing sequences retain their previous SDR appearance until this setting
changes. Legacy shaders use an explicit sRGB compatibility boundary in the
managed monitor. HDR video preview uses the original P010 surface even when
the project has SDR proxies enabled.

The input fixture checks that a one-stop exposure adjustment occurs before
the SDR view, then verifies undo, redo and reopening at the same pixel level.

## Sony camera input

Select a video in Library and use **Input color** in its asset details to override the source
interpretation. The three presets are **S-Log3 / S-Gamut3.Cine**, **S-Log3 /
S-Gamut3**, and **S-Log2 / S-Gamut**. The choice is stored with the asset and
applies in the source monitor, on the timeline, and in exports. **Auto** restores
metadata-based interpretation. The GPU decodes the logarithmic transfer, adapts
the gamut and white point to linear AP1, and applies the SDR view when needed.
Camera log sources use the original media to avoid baking the transform into a
proxy. Reference tests use synthetic video patches. The S-Log2 coefficients
follow the [OCIO ACES Sony CLF generator](https://opencolorio.readthedocs.io/projects/config-aces/en/stable/_modules/opencolorio_config_aces/clf/transforms/sony/generate.html).
