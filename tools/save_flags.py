"""Sets IESaveState flags (e.g. ClubsIntroducible IntroducedClubs) to true in the latest Azimuth save revision.
Usage: python save_flags.py Flag [Flag ...]   (game closed!)
Backs up the whole user save folder first; rewrites SaveState and its hash/size in `commit`."""
import hashlib, os, re, shutil, sys, time
user = r"C:\Users\alcmo\Downloads\GAMES\FH2\azimuth-windows\data\saves\player-1\User_9000000000000"
revs = os.path.join(user, "revisions")
latest = sorted(os.listdir(revs))[-1]
rev = os.path.join(revs, latest)
backup = os.path.join(os.path.dirname(user), "backup-" + time.strftime("%Y%m%d-%H%M%S"))
shutil.copytree(user, backup)
print("backup:", backup, "| revision:", latest)
path = os.path.join(rev, "data", "SaveState")
xml = open(path, "rb").read()
changed = xml
for name in sys.argv[1:]:
    changed, n = re.subn(rb'(<Property Name="' + name.encode() + rb'" Value=")false("/>)', rb"\1true\2", changed)
    print(name, "->", "true" if n else "(not false / not found)")
if changed == xml:
    sys.exit("nothing to change")
open(path, "wb").write(changed)
commit_path = os.path.join(rev, "commit")
commit = open(commit_path, "rb").read().decode()
digest = hashlib.sha256(changed).hexdigest()
lines = commit.split("\n")
for i, l in enumerate(lines):
    parts = l.split(" ")
    if len(parts) == 3 and parts[2] == "SaveState":
        lines[i] = f"{digest} {len(changed)} SaveState"
open(commit_path, "wb").write("\n".join(lines).encode())
print("commit updated:", digest, len(changed))
