# Numeric PID reference data

`fields.tsv` supplies 560 numeric fields across the 195 measurement-bearing PIDs
in [PIDS.MD](../../../PIDS.MD). ViewerUI uses this offline table to interpret raw
Service 01 positive response payloads. It never writes decoded values into the
recording or requires a live ECU/network connection to view them.

## Provenance and license

The main field definitions are adapted from
[OBDb/SAEJ1979, revision 06690177d2d5e11dbf82566297128d673b5a03a7](https://github.com/OBDb/SAEJ1979/blob/06690177d2d5e11dbf82566297128d673b5a03a7/signalsets/v3/default.json),
published by the OBDb contributors under CC BY-SA 4.0. The adapted table remains
under that license; see [LICENSE-OBDb](LICENSE-OBDb). The input SHA-256 is recorded
in the TSV header and checked by the generator. No upstream prose descriptions,
vehicle addressing, scheduling, or online lookup logic are imported.

Changes include selecting numeric measurements/counters, converting unit labels,
retaining sensor support/no-result flags as decode conditions, and omitting
boolean flags, compliance/fuel enums, and manufacturer-defined DEF dosing modes
from analog plotting. Hidden upstream numeric limits and torque points are retained.
NOx and particulate concentration units are made explicit. Oxygen sensor PIDs
14–1B, 24–2B and 34–3B use sensor ordinals rather than assuming a bank layout.

[DashLogic's detailed layouts](https://www.dashlogic.com/docs/technical/obdii_pids)
provide these supplements/corrections:

- Optional bank 3 fuel trims for 06/07 and secondary trims for banks 3/4 in 55–58.
- Turbo inlet pressures 6F, including the wide-range scaling flag, and MAP sensors 87.
- Exhaust temperature sensors 5–8 in 98/99.
- Injection timing 5D uses `raw / 128 - 210` degrees, correcting the inconsistent
  upstream offset of -38665. Independent engineering-value tests cover this case.
- Standard endpoints remain valid for RPM/speed maxima, zero fuel level and
  -40°C intake readings. Upstream display-specific null heuristics for those
  values are omitted; sensor support flags still apply.

The 14 PIDs without numeric fields are 03, 12, 13, 1C, 1D, 1E, 41, 51, 5F,
7D, 7E, 92, C8 and D8. They remain raw/categorical. Unverified slots are not decoded.

## Table format and regeneration

Each tab-separated row contains: PID, field key, display name, unit, starting bit,
bit width, signed flag, multiplier, divisor, additive offset, unavailable lower
threshold, unavailable upper threshold, support bit offsets (any set bit permits
the field), no-result bit offsets (any set bit suppresses the field), and an
optional 6F wide-range bit. Empty optional cells mean no such condition.

Bit offsets are MSB-first from data byte A, after response bytes `41 PID`.
The value is `signed_or_unsigned_raw * multiplier / divisor + offset`, with an
additional factor of eight when a 6F wide-range bit is set. No-result thresholds
are applied to the scaled value. Missing bytes produce no value; complete earlier
fields in a truncated block remain interpretable. Values are not clamped to
rounded reference maxima.

To regenerate from an explicitly downloaded copy of the pinned input:

```sh
python3 src/viewerui/pids/generate.py /path/to/default.json
```

The generator has no network access and rejects a different input revision.
Review reference changes and engineering-value tests before updating the pin.
`tests/pid_decoding.rs` checks coverage against PIDS.MD and representative formulas;
`tests/pid_recordings.rs` exercises every numeric field through read-only plotting.
