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
