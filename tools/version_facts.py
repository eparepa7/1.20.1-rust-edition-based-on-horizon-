"""Version facts shared by the native-version generator and port inspection tools."""


def pack_formats(info):
    """Return resource and data pack formats as (major, minor) pairs.

    Versions such as 1.20.1 store a single integer for each type; newer releases
    store separate major/minor numbers. Do not silently invent a missing major.
    """
    pack = info["pack_version"]

    def one(kind):
        if isinstance(pack, int) and not isinstance(pack, bool):
            major, minor = pack, 0
        elif isinstance(pack, dict):
            major = pack.get(f"{kind}_major", pack.get(kind))
            minor = pack.get(f"{kind}_minor", 0)
        else:
            raise ValueError("Unsupported pack_version format")
        if (type(major) is not int or type(minor) is not int
                or major < 0 or minor < 0):
            raise ValueError(f"Missing or invalid {kind} pack version")
        return major, minor

    return one("resource"), one("data")
