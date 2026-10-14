"""Builds the starting saves of new Infinity profiles: one per starter car, past the game's
intro and first championship, with online, clubs and rivals open.

Usage (Windows, game closed is not needed):
  python tools/make_starter_saves.py <source revision dir> <gamedbRC.slt> <out dir> [credits]

The source is an Azimuth save revision of a profile past the first championship
(data\\saves\\player-1\\User_<xuid>\\revisions\\<hex>). Each output is that revision with:
- Career_Garage holding only the starter car, stock (its row built from gamedbRC.slt the
  way the game writes it), the last used car; bought parts and change tracking cleared;
- ProfileData: Credits and MostValuableCar set;
- SaveState: IntroducedMultiplayer true (clubs and rivals are already introduced past the
  first championship);
- the commit's hashes and sizes updated.
It is written as the FH2SAVE1 bundle the server hands to a new profile (server/src/db/saves.rs,
hook/adapters/azimuth_cloud.c): <out>/starter-<car>.bundle and starter-<car>.commit (the
SHA-256 of the commit file). Every bundle is unpacked again and checked before it is kept."""
import ctypes, hashlib, os, re, sqlite3, struct, sys, tempfile, uuid

STARTERS = (289, 461, 1218)  # Camaro SS 69, Supra RZ 98, BMW Z4 11 (gamedb IEAutoShowCars)
FILES = ["BinarySaveState", "PlayerDatabase", "ProfileData", "SaveState", "TransactionLogFile", "VersionFlags"]
CONTAINER_SIZE = 6291464

PART_TABLES = {
    "Engine": "List_UpgradeEngine", "Drivetrain": "List_UpgradeDrivetrain", "CarBody": "List_UpgradeCarBody",
    "Motor": "List_UpgradeMotor", "Brakes": "List_UpgradeBrakes", "SpringDamper": "List_UpgradeSpringDamper",
    "AntiSwayFront": "List_UpgradeAntiSwayFront", "AntiSwayRear": "List_UpgradeAntiSwayRear",
    "TireCompound": "List_UpgradeTireCompound", "RearWing": "List_UpgradeRearWing",
    "RimSizeFront": "List_UpgradeRimSizeFront", "RimSizeRear": "List_UpgradeRimSizeRear",
    "Camshaft": "List_UpgradeEngineCamshaft", "Valves": "List_UpgradeEngineValves",
    "Displacement": "List_UpgradeEngineDisplacement", "PistonsCompression": "List_UpgradeEnginePistonsCompression",
    "FuelSystem": "List_UpgradeEngineFuelSystem", "Ignition": "List_UpgradeEngineIgnition",
    "Exhaust": "List_UpgradeEngineExhaust", "Intake": "List_UpgradeEngineIntake", "Flywheel": "List_UpgradeEngineFlywheel",
    "Manifold": "List_UpgradeEngineManifold", "RestrictorPlate": "List_UpgradeEngineRestrictorPlate",
    "OilCooling": "List_UpgradeEngineOilCooling", "SingleTurbo": "List_UpgradeEngineTurboSingle",
    "TwinTurbo": "List_UpgradeEngineTurboTwin", "QuadTurbo": "List_UpgradeEngineTurboQuad",
    "SuperchargerCSC": "List_UpgradeEngineCSC", "SuperchargerDSC": "List_UpgradeEngineDSC",
    "Intercooler": "List_UpgradeEngineIntercooler", "Clutch": "List_UpgradeDrivetrainClutch",
    "Transmission": "List_UpgradeDrivetrainTransmission", "Driveline": "List_UpgradeDrivetrainDriveline",
    "Differential": "List_UpgradeDrivetrainDifferential", "FrontBumper": "List_UpgradeCarBodyFrontBumper",
    "RearBumper": "List_UpgradeCarBodyRearBumper", "Hood": "List_UpgradeCarBodyHood",
    "SideSkirts": "List_UpgradeCarBodySideSkirt", "TireWidthFront": "List_UpgradeCarBodyTireWidthFront",
    "TireWidthRear": "List_UpgradeCarBodyTireWidthRear", "WeightReduction": "List_UpgradeCarBodyWeight",
    "ChassisStiffness": "List_UpgradeCarBodyChassisStiffness", "MotorParts": "List_UpgradeMotorParts",
}


class GameDb:
    def __init__(self, path):
        self.db = sqlite3.connect(f"file:{path}?mode=ro", uri=True)

    def cols(self, table):
        return [c[1] for c in self.db.execute(f'pragma table_info("{table}")')]

    def stock_id(self, table, key, value):
        cols = self.cols(table)
        key = next(c for c in cols if c.lower() == key.lower())
        extra = " and IsStock = 1" if "IsStock" in cols else " and Level = 0" if "Level" in cols else ""
        row = self.db.execute(f'select Id from "{table}" where "{key}" = ?{extra}', (value,)).fetchone()
        return row[0] if row else -1

    def stock_of(self, table, car, column):
        cols = self.cols(table)
        if column not in cols:
            return None
        extra = " and IsStock = 1" if "IsStock" in cols else ""
        row = self.db.execute(f'select "{column}" from "{table}" where Ordinal = ?{extra}', (car,)).fetchone()
        return row[0] if row else None

    def parts(self, car):
        keys = {
            "ordinal": car,
            "engineid": self.stock_of("List_UpgradeEngine", car, "EngineID"),
            "drivetrainid": self.stock_of("List_UpgradeDrivetrain", car, "DrivetrainID"),
            "carbodyid": self.stock_of("List_UpgradeCarBody", car, "CarBodyID"),
            "motorid": self.stock_of("List_UpgradeMotor", car, "MotorID"),
        }
        out = {}
        for column, table in PART_TABLES.items():
            cols = [c.lower() for c in self.cols(table)]
            key = next(k for k in keys if k in cols)
            out[column] = self.stock_id(table, key, keys[key])
        return out

    def car(self, car):
        cur = self.db.execute("select * from Data_Car where Id = ?", (car,))
        names = [d[0] for d in cur.description]
        return dict(zip(names, cur.fetchone()))

    def colour(self, car):
        row = self.db.execute("select ColourId from IEAutoShowCars where CarId = ?", (car,)).fetchone()
        return row[0] if row else 0


def garage_row(columns, template, game, car):
    """The stock starter car, as the game writes a new car (checked against its own rows)."""
    d = game.car(car)
    row = dict(zip(columns, template))
    row.update(game.parts(car))
    row.update({
        "Id": 1, "CarId": car, "PerformanceIndex": d["PerformanceIndex"], "ClassID": d["ClassID"], "PartsValue": 0,
        "SpeedRating": d["SpeedRating"], "AccelerationRating": d["AccelerationRating"], "LaunchRating": d["LaunchRating"],
        "BrakingRating": d["BrakingRating"], "HandlingRating": d["HandlingRating"], "CurbWeight": d["CurbWeight"],
        "WeightDistribution": d["WeightDistribution"], "AspirationTypeId": d["AspirationTypeId"],
        "SimPeakPower": d["SimPeakPower"], "SimPeakAngVel": d["SimPeakAngVel"], "SimPeakTorque": d["SimPeakTorque"],
        "SimPeakTorqueAngVel": d["SimPeakTorqueAngVel"], "SimRedlineAngVel": d["SimRedlineAngVel"],
        "PeakIntakePSI": 0.0, "TopSpeed": 0.0, "OriginalOwner": "", "CarGroup": None,
        "DefaultManufacturerColorIndex": game.colour(car), "LiveryFileName": "", "TuneFileName": "",
        "VersionedTuneId": -1, "VersionedLiveryId": -1, "Guid": str(uuid.uuid4()),
        "Thumbnail": f"game:\\Media\\Stripped\\RC0\\Thumbnail_{car}.png", "SharedID": 0,
    })
    for stat in ("DistanceDriven", "TimeDriven", "TotalWinnings", "TotalRepairs", "NumVictories", "NumPodiums",
                 "NumRaces", "NumTimesSold", "TimeDrivenInRoadTrips", "CurOwnerNumRaces", "CurOwnerWinnings",
                 "NumSkillPointsEarned"):
        row[stat] = 0
    row["NumOwners"] = 1
    for c in columns:
        if c.startswith("Tuning_"):
            row[c] = -1.0
    return [row[c] for c in columns]


def edit_database(container, game, car):
    size = struct.unpack_from("<I", container, 4)[0]
    assert container[:4] == b"ssmc" and container[8:24] == b"SQLite format 3\0"
    path = os.path.join(tempfile.gettempdir(), f"starter-{car}-{os.getpid()}.sqlite")
    open(path, "wb").write(container[8:8 + size])
    db = sqlite3.connect(path)
    db.execute("pragma journal_mode = delete")
    columns = [c[1] for c in db.execute("pragma table_info(Career_Garage)")]
    template = db.execute("select * from Career_Garage where CarId = 461").fetchone() or db.execute(
        "select * from Career_Garage").fetchone()
    row = garage_row(columns, template, game, car)
    db.execute("delete from Career_Garage")
    db.execute(f"insert into Career_Garage values ({','.join('?' * len(columns))})", row)
    db.execute("delete from Career_PurchasedParts")
    db.execute("delete from ChangeTracking")
    db.execute("update LastUsedGarageId set LastUsedGarageId = 1")
    db.commit()
    assert db.execute("pragma integrity_check").fetchone()[0] == "ok"
    db.close()
    data = open(path, "rb").read()
    os.remove(path)
    out = b"ssmc" + struct.pack("<I", len(data)) + data
    assert len(out) <= CONTAINER_SIZE
    return out + b"\0" * (CONTAINER_SIZE - len(out))


def set_profile_int(profile, name, value):
    marker = name.encode() + b" \0\0\0\0\x03\0\0\0"
    at = profile.find(marker)
    assert at > 0 and profile.find(marker, at + 1) < 0, name
    at += len(marker)
    return profile[:at] + struct.pack("<i", value) + profile[at + 4:]


def set_flag(state, name, value):
    changed, n = re.subn(rb'(<Property Name="' + name.encode() + rb'" Value=")(?:true|false)("/>)',
                         rb"\g<1>" + (b"true" if value else b"false") + rb"\g<2>", state)
    assert n == 1, name
    return changed


def commit_with(commit, files):
    lines = commit.decode().split("\n")
    for i, line in enumerate(lines):
        parts = line.split(" ")
        if len(parts) == 3 and parts[2] in files:
            data = files[parts[2]]
            lines[i] = f"{hashlib.sha256(data).hexdigest()} {len(data)} {parts[2]}"
    return "\n".join(lines).encode()


# ntdll XPRESS Huffman, as the hook (azimuth_cloud.c)
ntdll = ctypes.WinDLL("ntdll")
FORMAT = 4


def workspace():
    a, b = ctypes.c_ulong(), ctypes.c_ulong()
    assert ntdll.RtlGetCompressionWorkSpaceSize(FORMAT, ctypes.byref(a), ctypes.byref(b)) == 0
    return ctypes.create_string_buffer(a.value)


WS = workspace()


def compress(data):
    out = ctypes.create_string_buffer(len(data))
    final = ctypes.c_ulong()
    status = ntdll.RtlCompressBuffer(FORMAT, data, len(data), out, len(data), 4096, ctypes.byref(final), WS)
    if status != 0 or not final.value or final.value >= len(data):
        return 0, data
    return 1, out.raw[:final.value]


def decompress(data, size):
    out = ctypes.create_string_buffer(size)
    final = ctypes.c_ulong()
    assert ntdll.RtlDecompressBufferEx(FORMAT, out, size, data, len(data), ctypes.byref(final), WS) == 0
    assert final.value == size
    return out.raw


def bundle(revision, files):
    out = bytearray(b"FH2SAVE1" + struct.pack("<QI", revision, len(files)))
    for name, data in files:
        method, stored = compress(data) if len(data) >= 64 else (0, data)
        out += struct.pack("<H", len(name)) + name.encode() + struct.pack("<IIB", len(data), len(stored), method) + stored
    return bytes(out)


def unbundle(data):
    assert data[:8] == b"FH2SAVE1"
    _, count = struct.unpack_from("<QI", data, 8)
    at, files = 20, {}
    for _ in range(count):
        (n,) = struct.unpack_from("<H", data, at)
        name = data[at + 2:at + 2 + n].decode()
        raw, stored, method = struct.unpack_from("<IIB", data, at + 2 + n)
        at += 2 + n + 9
        body = data[at:at + stored]
        files[name] = decompress(body, raw) if method == 1 else body
        at += stored
    assert at == len(data)
    return files


def main():
    source, gamedb, out_dir = sys.argv[1], sys.argv[2], sys.argv[3]
    credits = int(sys.argv[4]) if len(sys.argv) > 4 else 50000
    game = GameDb(gamedb)
    original = {name: open(os.path.join(source, "data", name), "rb").read() for name in FILES}
    commit = open(os.path.join(source, "commit"), "rb").read()
    os.makedirs(out_dir, exist_ok=True)
    for car in STARTERS:
        files = dict(original)
        files["PlayerDatabase"] = edit_database(files["PlayerDatabase"], game, car)
        profile = set_profile_int(files["ProfileData"], "Credits", credits)
        files["ProfileData"] = set_profile_int(profile, "MostValuableCar", car)
        files["SaveState"] = set_flag(files["SaveState"], "IntroducedMultiplayer", True)
        new_commit = commit_with(commit, files)
        packed = bundle(1, [("commit", new_commit)] + [("data/" + n, files[n]) for n in FILES])
        # check: unpack, hashes as the commit lists them, the garage as intended
        back = unbundle(packed)
        assert back["commit"] == new_commit
        for n in FILES:
            assert back["data/" + n] == files[n]
            assert f"{hashlib.sha256(files[n]).hexdigest()} {len(files[n])} {n}".encode() in new_commit
        open(os.path.join(out_dir, f"starter-{car}.bundle"), "wb").write(packed)
        open(os.path.join(out_dir, f"starter-{car}.commit"), "w").write(hashlib.sha256(new_commit).hexdigest())
        print(f"starter {car}: {len(packed)} bytes, commit {hashlib.sha256(new_commit).hexdigest()[:16]}")


if __name__ == "__main__":
    main()
