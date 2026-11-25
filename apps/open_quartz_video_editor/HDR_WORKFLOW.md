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

In the MP4 export dialog select **HDR10 PQ · BT.2020 / 1000 nits** for a
single visible video track. This declares a BT.2020/D65 1000 cd/m² mastering
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
PQ sample values.
