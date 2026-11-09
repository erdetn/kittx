"""Generate the offline numeric table from a downloaded, pinned OBDb JSON file.

Usage: python3 src/viewerui/pids/generate.py /path/to/default.json
The generator performs no network access. See README.md for provenance/overrides.
"""
import hashlib
import json
from pathlib import Path
import re
import sys

REVISION = "06690177d2d5e11dbf82566297128d673b5a03a7"
source = Path(sys.argv[1]).read_bytes()
assert hashlib.sha256(source).hexdigest() == "25aaa40bf024f813f470844a71c4a1ee7f8bdfa61ff7adc5b6f4e60891b7ad33", "Expected the pinned OBDb input; review changes before updating"
commands = json.loads(source, strict=False)["commands"]
units = {
    "percent": "%", "celsius": "degC", "scalar": "count", "seconds": "s",
    "kilopascal": "kPa", "kilowattHours": "kWh", "volts": "V", "amps": "A",
    "kilowatts": "kW", "kilometers": "km", "hours": "h", "rpm": "rpm",
    "gramsPerSecond": "g/s", "milliamps": "mA", "newtonMeters": "N·m",
    "kilogramsPerHour": "kg/h", "minutes": "min", "gramsPerLiter": "g/L",
    "litersPerHour": "L/h", "kilometersPerHour": "km/h", "degrees": "deg",
    "milligramsPerStroke": "mg/stroke", "liters": "L",
}
rows = []


def append(pid, key, name, unit, bit, width, signed=False, mul=1, div=1,
           add=0, nullmin="", nullmax="", support=(), unavailable=(), wide=""):
    rows.append([f"{pid:02X}", key, name, unit, bit, width, int(signed), mul, div,
                 add, nullmin, nullmax, ",".join(map(str, support)),
                 ",".join(map(str, unavailable)), wide])


for command in commands:
    pid = int(command["cmd"]["01"], 16)
    signals = command["signals"]
    by_id = {s["id"]: s for s in signals}
    for s in signals:
        key, fmt = s["id"], dict(s["fmt"])
        # Upstream's -38665 is inconsistent with the documented -210° offset.
        if pid == 0x5D:
            fmt["add"] = -210
        # Do not import vehicle/UI-specific missing-value heuristics for valid
        # standard endpoints (full RPM/speed range, empty fuel, -40°C intake).
        if pid in (0x0C, 0x0D):
            fmt.pop("nullmax", None)
        if pid in (0x2F, 0x68):
            fmt.pop("nullmin", None)
        # Retain numeric limits/counters even if upstream hides them in its UI.
        # Gear is a numeric quantity; manufacturer-defined DEF modes are not.
        if (fmt["len"] <= 1 or "map" in fmt or fmt.get("unit") == "hex"
                or key in ("DEFA_MODE", "DEFB_MODE")):
            continue
        unit = "gear" if key in ("GEAR_RCMD", "GEAR_ACT") else units[fmt["unit"]]
        if "LAMBDA" in key or key in ("EQ_RAT", "MAX_EQ_RAT", "GEAR_RAT"):
            unit = "ratio"
        if pid in (0x83, 0xA1, 0xA7, 0xA8):
            unit = "ppm"
        if pid == 0x86:
            unit = "mg/m³"
        support_key = key + "_SUP"
        aliases = {"TAC_A_CMD": "CTAC_A_SUP", "TP_A_REL": "RTP_A_SUP",
                   "TAC_B_CMD": "CTAC_B_SUP", "TP_B_REL": "RTP_B_SUP",
                   "GEAR_RCMD": "GEAR_SUP"}
        if key in aliases:
            support_key = aliases[key]
        if re.fullmatch(r"AECD\d+_TIME[12]", key):
            support_key = key.rsplit("_", 1)[0] + "_SUP"
        support = [by_id[support_key]["fmt"].get("bix", 0)] if support_key in by_id else []
        unavailable = [by_id[key + "_NA"]["fmt"].get("bix", 0)] if key + "_NA" in by_id else []
        name = s["name"]
        # Sensor ordinal is unambiguous; physical bank layout is advertised separately.
        for start, end in ((0x14, 0x1B), (0x24, 0x2B), (0x34, 0x3B)):
            if start <= pid <= end:
                detail = ("short-term fuel trim" if key.startswith("SHRTFT") else
                          "equivalence ratio" if "LAMBDA" in key else
                          "current" if fmt["unit"] == "milliamps" else "voltage")
                name = f"Oxygen sensor {pid - start + 1} {detail}"
        append(pid, key, name, unit, fmt.get("bix", 0), fmt["len"],
               fmt.get("sign", False), fmt.get("mul", 1), fmt.get("div", 1),
               fmt.get("add", 0), fmt.get("nullmin", ""), fmt.get("nullmax", ""),
               support, unavailable)

# Documented fields missing from the pinned upstream table (DashLogic layouts).
for pid, key in ((0x06, "SHRTFT3"), (0x07, "LONGFT3")):
    append(pid, key, ("Short" if pid == 6 else "Long") + " term fuel trim (bank 3)",
           "%", 8, 8, mul=100, div=128, add=-100)
for pid, key, term, bank in ((0x55,"STSO2FT3","Short",3),
                            (0x56,"LGSO2FT3","Long",3),
                            (0x57,"STSO2FT4","Short",4),
                            (0x58,"LGSO2FT4","Long",4)):
    append(pid, key, f"{term} term secondary oxygen sensor trim bank {bank}",
           "%", 8, 8, mul=100, div=128, add=-100)
for index, sensor in enumerate("AB"):
    append(0x6F, f"TCIP_{sensor}", f"Turbocharger compressor inlet pressure {sensor}",
           "kPa", 8 + index * 8, 8, support=[7 - index, 5 - index], wide=5 - index)
    append(0x87, f"MAP_{sensor}", f"Intake manifold absolute pressure {sensor}",
           "kPa", 8 + index * 16, 16, div=32, support=[7 - index])
for pid, bank in ((0x98, 1), (0x99, 2)):
    for index in range(4):
        append(pid, f"EGT{bank}{index + 5}",
               f"Exhaust gas temperature bank {bank} sensor {index + 5}",
               "degC", 8 + index * 16, 16, div=10, add=-40, support=[7 - index])

rows.sort(key=lambda r: (int(r[0], 16), int(r[4]), r[1]))
assert len({(r[0], r[1]) for r in rows}) == len(rows)
assert all(0 < int(r[5]) <= 32 and float(r[8]) != 0 for r in rows)
out = Path(__file__).parent
header = (f"# Numeric PID fields; derived from OBDb/SAEJ1979 {REVISION}\n"
          f"# Input SHA256: {hashlib.sha256(source).hexdigest()}\n"
          "# License/provenance and manual supplements: README.md\n"
          "# pid\tkey\tname\tunit\tbit\twidth\tsigned\tmul\tdiv\tadd\tnullmin\tnullmax\tsupport_bits_any\tunavailable_bits\twide_range_bit\n")
(out / "fields.tsv").write_text(header + "".join("\t".join(map(str, row)) + "\n" for row in rows))
print(f"Generated {len(rows)} numeric fields across {len({r[0] for r in rows})} PIDs")
