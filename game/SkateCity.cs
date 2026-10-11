//
// SkateCity: Skate 3's skating for Batman: Arkham City.
//
// The skate engine is 64-bit and Arkham City is 32-bit, so the engine runs
// in a helper program (Binaries\Win32\SkateCity\SkateCity.exe). This script
// starts it and talks to it every frame over shared memory: it reports
// Batman and casts the rays the helper asks for, and the helper answers with
// where Batman, his bones, the board and the camera should be.
//
// Press K (or click both sticks) to start and stop skating.
//

using System.IO.MemoryMappedFiles;
using System.Numerics;
using System.Runtime.InteropServices;
using System.Text;
using BmSDK;
using BmSDK.BmGame;
using BmSDK.Engine;

[Script]
public class SkateCityScript : Script
{
    public override void OnEnterGame() => SkateLink.Instance.EnterGame();

    public override void OnTick() => SkateLink.Instance.Tick();

    public override void OnKeyDown(Keys key) => SkateLink.Instance.KeyDown(key);

    public override void OnUnload() => SkateLink.Shutdown();
}

/// <summary>
/// The HUD hook: while skating, the trick line is drawn on screen.
/// </summary>
public static class SkateCityRedirects
{
    [Redirect(typeof(RHUD), nameof(RHUD.PostRender))]
    static void PostRender(RHUD self)
    {
        self.PostRender();
        try
        {
            SkateLink.Mark(30);
            SkateLink.DrawHud(self);
            SkateLink.Mark(31);
        }
        catch (Exception e)
        {
            Debug.Log($"SkateCity HUD: {e.Message}");
        }
    }
}

public sealed unsafe class SkateLink
{
    static SkateLink? s_instance;
    public static SkateLink Instance => s_instance ??= new SkateLink();

    // Shared memory layout: must match ipc.rs in the helper.
    const int Size = 1 << 20;
    const uint Magic = 0x534B4331;
    const int MaxRays = 4096, MaxBones = 512, NameLen = 64;
    const int ReqHits = 0x1000, ReqNames = 0x20000, ReqParents = 0x28000, ReqLocals = 0x28800;
    const int Rep = 0x40000, RepRays = 0x41000, RepBones = 0x60000;
    // Request fields.
    const int RqMagic = 0, RqSeq = 4, RqPid = 8, RqFlags = 12, RqDt = 16, RqLoc = 20, RqRot = 32, RqHalf = 44,
        RqMatrix = 48, RqBatch = 112, RqRayCount = 116, RqSkelSeq = 120, RqBoneCount = 124, RqToggles = 128, RqWorld = 132, RqStep = 136, RqFrames = 140;
    // Reply fields (offsets from Rep).
    const int RpSeq = 0, RpState = 4, RpFlags = 8, RpPawnLoc = 12, RpPawnRot = 24, RpCamLoc = 36, RpCamRot = 48,
        RpBatch = 64, RpRayCount = 68, RpBoneCount = 72, RpMsgSeq = 76, RpBoardCount = 80, RpBoard = 84,
        RpMsg = 372, RpScore = 628, RpToggleKey = 760, RpGlideVel = 764, RpGlideSeq = 776, RpGlideEndSeq = 780,
        RpHudPoints = 784, RpHudTotal = 788, RpResultSeq = 792, RpResultKind = 796, RpResultPoints = 800, RpResultText = 804, RpBoardMore = 932,
        RpSoundCount = 1004, RpSounds = 1012, SoundSlots = 8;
    const uint ResultClean = 1, ResultSketchy = 3, ResultBailed = 4, ResultSlam = 5;
    const uint PawnValid = 1, SkeletonValid = 2;
    const uint SetPawn = 1, SetCamera = 2, SetBones = 4, SetBoard = 8, Skating = 16;
    const uint StateFailed = 4;

    // Offsets into engine objects (from BmSDK's generated classes).
    const int ComponentLocalToWorld = 112;
    const int SkelSpaceBases = 600, SkelLocalAtoms = 612;

    static bool CameraActive;
    static string s_message = "";
    static long s_messageAt;
    static string s_score = "";
    // The trick display.
    static bool s_hudSkating, s_resultSynced;
    static uint s_points, s_total, s_resultSeq, s_resultKind, s_resultPoints;
    static string s_resultText = "";
    static long s_resultAt = -100000;

    MemoryMappedFile? _map;
    MemoryMappedViewAccessor? _view;
    byte* _base;
    EventWaitHandle? _requests, _replies;
    System.Diagnostics.Process? _helper;
    bool _startFailed;
    uint _seq, _skelSeq, _toggles, _answeredBatch, _answeredCount, _lastMsgSeq, _world;
    IntPtr _skelMesh, _skelAsset;
    int _boneCount;
    int _toggleKey = 0x4B;
    bool _skating;
    Vector3 _lastPawnLoc;
    EPhysicsSave _saved;
    int _frames, _lateReplies, _bonesMismatched;
    DynamicCameraActor? _cam;
    int _errors;
    readonly DynamicSMActor_Spawnable?[] _board = new DynamicSMActor_Spawnable?[10];
    StaticMesh? _cube, _round;
    Vector3 _roundExtent;
    readonly MaterialInterface?[] _boardMaterials = new MaterialInterface?[3];
    // Sounds: the helper's running count of sound events, and the game's
    // cue for each kind (1 pop, 2 land, 3 grind, 4 hit, 5 hard hit, 6 break).
    uint _soundSeen;
    bool _soundSynced, _soundsSearched;
    readonly SoundCue?[] _cues = new SoundCue?[7];
    bool _cubeSearched;
    bool _badCountLogged;
    IntPtr _stableMesh;
    IntPtr _skatePawn, _skateMesh;
    int _stableFrames;
    Vector3 _cubeExtent;
    // Gliding: Batman off the board, in the game's own glide.
    bool _glideSynced, _gliding;
    uint _glideSeq, _glideEndSeq;
    int _glideFrames, _glideTries, _landedFrames, _capeOpenFrame = -1;
    IntPtr _glidePawn;

    struct EPhysicsSave
    {
        public Actor.EPhysics Physics;
        public bool CollideActors, BlockActors, CollideWorld;
    }

    public static void Shutdown()
    {
        var link = s_instance;
        s_instance = null;
        if (link == null)
        {
            return;
        }
        try
        {
            link.StopSkating();
            foreach (var part in link._board)
            {
                if (part != null && part.IsValid)
                {
                    part.Destroy();
                }
            }
            if (link._cam != null && link._cam.IsValid)
            {
                link._cam.Destroy();
            }
            link._helper?.Kill();
        }
        catch (Exception e)
        {
            Debug.Log($"SkateCity shutdown: {e.Message}");
        }
    }

    public void EnterGame()
    {
        // A new world: the old board actors went with the old one.
        Array.Clear(_board);
        _cam = null;
        _skating = false;
        CameraActive = false;
        _skelMesh = IntPtr.Zero;
        _world++;
        Start();
    }

    public void KeyDown(Keys key)
    {
        if ((int)key == _toggleKey)
        {
            _toggles++;
        }
    }

    void Start()
    {
        if (_base != null || _startFailed)
        {
            return;
        }
        try
        {
            var pid = (uint)Environment.ProcessId;
            var name = $"Local\\SkateCity_{pid}";
            _map = MemoryMappedFile.CreateOrOpen(name, Size);
            _view = _map.CreateViewAccessor(0, Size);
            byte* p = null;
            _view.SafeMemoryMappedViewHandle.AcquirePointer(ref p);
            _base = p + _view.PointerOffset;
            new Span<byte>(_base, Size).Clear();
            W32(RqMagic, Magic);
            W32(RqPid, pid);
            _requests = new EventWaitHandle(false, EventResetMode.AutoReset, name + "_req");
            _replies = new EventWaitHandle(false, EventResetMode.AutoReset, name + "_rep");

            foreach (var old in System.Diagnostics.Process.GetProcessesByName("SkateCity"))
            {
                try { old.Kill(); } catch { }
            }
            var binaries = Path.GetDirectoryName(Environment.ProcessPath) ?? ".";
            var exe = Path.Combine(binaries, "SkateCity", "SkateCity.exe");
            if (!File.Exists(exe))
            {
                Debug.LogError($"SkateCity: {exe} is missing - run build.bat in the SkateCity folder");
                _startFailed = true;
                return;
            }
            _helper = System.Diagnostics.Process.Start(new System.Diagnostics.ProcessStartInfo(exe, pid.ToString())
            {
                UseShellExecute = false,
                CreateNoWindow = true,
                WorkingDirectory = Path.GetDirectoryName(exe),
            });
            Debug.Log($"SkateCity: helper started (process {_helper?.Id}); loading Skate 3 data");
            Show("SkateCity: loading Skate 3...");
        }
        catch (Exception e)
        {
            Debug.LogError($"SkateCity could not start: {e}");
            _startFailed = true;
        }
    }

    uint R32(int off) => *(uint*)(_base + off);
    int RI32(int off) => *(int*)(_base + off);
    float RF(int off) => *(float*)(_base + off);
    void W32(int off, uint v) => *(uint*)(_base + off) = v;
    void WI32(int off, int v) => *(int*)(_base + off) = v;
    void WF(int off, float v) => *(float*)(_base + off) = v;
    Vector3 RV(int off) => new(RF(off), RF(off + 4), RF(off + 8));
    void WV(int off, Vector3 v) { WF(off, v.X); WF(off + 4, v.Y); WF(off + 8, v.Z); }
    Rotator RR(int off) => Rotator.FromRuu(RI32(off), RI32(off + 4), RI32(off + 8));

    /// Breadcrumbs for the helper's log, in case the game dies mid-frame.
    static void Step(uint step)
    {
        var link = s_instance;
        if (link != null && link._base != null)
        {
            *(uint*)(link._base + RqStep) = step;
        }
    }

    public static void Mark(uint step) => Step(step);

    static int Ruu(float degrees) => (int)MathF.Round(degrees * Rotator.DegreesToRuuFactor);

    string ReadText(int off, int max)
    {
        var span = new ReadOnlySpan<byte>(_base + off, max);
        var end = span.IndexOf((byte)0);
        return Encoding.UTF8.GetString(end < 0 ? span : span[..end]);
    }

    public void Tick()
    {
        try
        {
            TickInner();
        }
        catch (Exception e)
        {
            if (_errors++ % 300 == 0)
            {
                Debug.LogError($"SkateCity: {e}");
            }
        }
    }

    void TickInner()
    {
        Step(1);
        if (_base == null)
        {
            Start();
            if (_base == null)
            {
                return;
            }
        }
        _frames++;
        W32(RqFrames, (uint)_frames);
        Step(2);

        RPawnPlayer? pawn = null;
        try
        {
            pawn = Game.GetPlayerPawn();
        }
        catch
        {
        }
        var valid = pawn != null && pawn.IsValid;
        var mesh = valid ? pawn!.Mesh : null;
        valid &= mesh != null && mesh.IsValid;

        uint flags = 0;
        if (valid)
        {
            flags |= PawnValid;
            Step(3);
            var loc = pawn!.Location;
            var rot = pawn.Rotation;
            WV(RqLoc, loc);
            WI32(RqRot, Ruu(rot.Pitch));
            WI32(RqRot + 4, Ruu(rot.Yaw));
            WI32(RqRot + 8, Ruu(rot.Roll));
            Step(4);
            var cylinder = pawn.CylinderComponent;
            WF(RqHalf, cylinder != null && cylinder.IsValid ? cylinder.CollisionHeight : 90f);
            Step(5);
            Buffer.MemoryCopy((void*)(mesh!.Ptr + ComponentLocalToWorld), _base + RqMatrix, 64, 64);
            if (SendSkeleton(mesh))
            {
                flags |= SkeletonValid;
            }
        }
        WF(RqDt, Math.Clamp(Game.GetDeltaTime(), 0f, 0.1f));
        W32(RqFlags, flags);
        W32(RqBatch, _answeredBatch);
        W32(RqRayCount, _answeredCount);
        W32(RqSkelSeq, _skelSeq);
        W32(RqBoneCount, (uint)_boneCount);
        W32(RqToggles, _toggles);
        W32(RqWorld, _world);
        _seq++;
        W32(RqSeq, _seq);
        Step(9);
        _requests!.Set();

        // The helper usually answers within a millisecond or two.
        // Events can hold a stale signal from a reply that came in late, so
        // wait on the sequence number itself.
        var deadline = Environment.TickCount64 + (_skating ? 12 : 4);
        while (R32(Rep + RpSeq) != _seq)
        {
            var left = deadline - Environment.TickCount64;
            if (left <= 0 || !_replies!.WaitOne((int)left))
            {
                break;
            }
        }
        var answered = R32(Rep + RpSeq) == _seq;
        if (!answered)
        {
            _lateReplies++;
            if (_skating && _lateReplies % 300 == 1)
            {
                Debug.Log($"SkateCity: {_lateReplies} late replies so far");
            }
        }
        if (_helper != null && _helper.HasExited)
        {
            Debug.LogError("SkateCity: the helper stopped - see Binaries\\Win32\\SkateCity\\SkateCity.log");
            _helper = null;
            StopSkating();
            return;
        }
        if (answered)
        {
            Step(10);
            Apply(valid ? pawn : null, valid ? mesh : null);
        }
        Step(20);
    }

    /// Sends Batman's bone names and parents once per mesh, and his current
    /// local bone transforms every frame.
    bool SendSkeleton(SkeletalMeshComponent mesh)
    {
        Step(6);
        var count = *(int*)(mesh.Ptr + SkelLocalAtoms + 4);
        var asset = mesh.SkeletalMesh;
        var assetPtr = asset != null ? asset.Ptr : IntPtr.Zero;
        if (count <= 0 || count > MaxBones)
        {
            if (!_badCountLogged)
            {
                _badCountLogged = true;
                Debug.Log($"SkateCity: Batman's mesh reports {count} bones; can't pose that");
            }
            return false;
        }
        // Batman's mesh is still being built for a moment after a level
        // loads, and asking for bone names then crashes the game: wait until
        // the same mesh has been in the world for a few seconds and its bone
        // arrays agree with each other.
        if (mesh.Ptr != _stableMesh)
        {
            _stableMesh = mesh.Ptr;
            _stableFrames = 0;
        }
        _stableFrames++;
        var spaceCount = *(int*)(mesh.Ptr + SkelSpaceBases + 4);
        var spaceData = *(IntPtr*)(mesh.Ptr + SkelSpaceBases);
        var localData = *(IntPtr*)(mesh.Ptr + SkelLocalAtoms);
        if (asset == null || !asset.IsValid || spaceCount != count || spaceData == IntPtr.Zero || localData == IntPtr.Zero)
        {
            _stableFrames = 0;
            return false;
        }
        var needNames = mesh.Ptr != _skelMesh || assetPtr != _skelAsset || count != _boneCount;
        if (needNames && _stableFrames < 240)
        {
            return false;
        }
        if (needNames)
        {
            _skelMesh = mesh.Ptr;
            _skelAsset = assetPtr;
            _boneCount = count;
            Step(7);
            // The game's GetBoneName crashes when a script calls it, so the
            // names and parents come straight from the mesh's reference
            // skeleton in memory.
            var names = new string[count];
            var parents = new int[count];
            if (!ReadRefSkeleton(asset, count, names, parents))
            {
                _skelMesh = IntPtr.Zero;
                _stableFrames = -100000;
                Debug.Log("SkateCity: couldn't find Batman's skeleton - skating will work but he won't be posed");
                return false;
            }
            Step(17);
            for (var i = 0; i < count; i++)
            {
                var bytes = Encoding.ASCII.GetBytes(names[i]);
                var slot = new Span<byte>(_base + ReqNames + i * NameLen, NameLen);
                slot.Clear();
                bytes.AsSpan(0, Math.Min(bytes.Length, NameLen - 1)).CopyTo(slot);
                WI32(ReqParents + i * 4, parents[i] == i ? -1 : parents[i]);
            }
            _skelSeq++;
            Debug.Log($"SkateCity: Batman's mesh {asset?.Name} has {count} bones ({spaceCount} in space), first {names[0]}");
        }
        Step(8);
        var locals = *(IntPtr*)(mesh.Ptr + SkelLocalAtoms);
        if (locals == IntPtr.Zero)
        {
            return false;
        }
        Buffer.MemoryCopy((void*)locals, _base + ReqLocals, MaxBones * 32, count * 32);
        return true;
    }

    static delegate* unmanaged[Stdcall]<void*, nuint, int> s_isBadReadPtr;

    /// True if `size` bytes at `p` can be read without crashing the game.
    static bool Readable(void* p, int size)
    {
        if (p == null || size <= 0)
        {
            return false;
        }
        if (s_isBadReadPtr == null)
        {
            var kernel = NativeLibrary.Load("kernel32.dll");
            s_isBadReadPtr = (delegate* unmanaged[Stdcall]<void*, nuint, int>)NativeLibrary.GetExport(kernel, "IsBadReadPtr");
        }
        return s_isBadReadPtr(p, (nuint)size) == 0;
    }

    /// A name from the game's name table, or null if the index isn't valid.
    static string? NameAt(int index)
    {
        var table = (byte*)MemUtil.GetPointer<byte>(GameDefine.Current.GNames);
        if (!Readable(table, 12))
        {
            return null;
        }
        var data = *(IntPtr**)table;
        var num = *(int*)(table + 4);
        if (index < 0 || index >= num || !Readable(data + index, 4))
        {
            return null;
        }
        var entry = (byte*)data[index];
        if (!Readable(entry + 16, 4))
        {
            return null;
        }
        return Marshal.PtrToStringUni((IntPtr)(entry + 16));
    }

    /// Finds the SkeletalMesh's reference skeleton (an array of bones, each
    /// starting with its name and holding its parent's index) by looking for
    /// an array of `count` elements whose names are all valid and whose
    /// parent indices all point backwards.
    static bool ReadRefSkeleton(SkeletalMesh asset, int count, string[] names, int[] parents)
    {
        const int Reach = 0x2000;
        var obj = (byte*)asset.Ptr;
        var candidates = new List<string>();
        for (var off = 0x3C; off < Reach - 12; off += 4)
        {
            if (!Readable(obj + off, 12))
            {
                break;
            }
            var data = *(byte**)(obj + off);
            var num = *(int*)(obj + off + 4);
            var max = *(int*)(obj + off + 8);
            if (num != count || max < num || max > num * 4 + 64 || !Readable(data, 72))
            {
                continue;
            }
            candidates.Add($"+0x{off:X}: {Hex(data, 16)} | {Hex(data + 52, 4)}");
            for (var stride = 16; stride <= 160; stride += 4)
            {
                if (!Readable(data, stride * count))
                {
                    continue;
                }
                for (var nameOff = 0; nameOff <= stride - 8; nameOff += 4)
                {
                    if (!NamesAt(data, stride, nameOff, count, names))
                    {
                        continue;
                    }
                    // The parent index: an int field that is 0 or -1 for the
                    // root, smaller than its own index for every other bone,
                    // and that puts each Biped calf under its thigh.
                    for (var field = 0; field <= stride - 4; field += 4)
                    {
                        if (field == nameOff || field == nameOff + 4)
                        {
                            continue;
                        }
                        var fits = true;
                        for (var i = 0; i < count && fits; i++)
                        {
                            var p = *(int*)(data + i * stride + field);
                            fits = i == 0 ? (p == 0 || p == -1) : (p >= 0 && p < i);
                        }
                        if (!fits)
                        {
                            continue;
                        }
                        for (var i = 0; i < count; i++)
                        {
                            parents[i] = i == 0 ? -1 : *(int*)(data + i * stride + field);
                        }
                        if (!LooksLikeBiped(names, parents))
                        {
                            continue;
                        }
                        Debug.Log($"SkateCity: reference skeleton found at +0x{off:X} (bone size {stride}, name at +{nameOff}, parent at +{field}); bones {names[0]}, {names[1]}, {names[Math.Min(2, count - 1)]}");
                        return true;
                    }
                    Debug.Log($"SkateCity: bone names found at +0x{off:X} (size {stride}, name at +{nameOff}) but no parent field");
                }
            }
        }
        Debug.Log($"SkateCity: skeleton search: {count} bones, mesh {asset.Name}, {candidates.Count} arrays of that size: {string.Join(" ; ", candidates.Take(8))}");
        return false;
    }

    /// Reads `count` names at `nameOff` in elements of `stride` bytes; true
    /// if they are all real names and include Biped bones.
    static bool NamesAt(byte* data, int stride, int nameOff, int count, string[] names)
    {
        var bip = false;
        for (var i = 0; i < count; i++)
        {
            var at = data + i * stride + nameOff;
            var name = NameAt(*(int*)at);
            var number = *(int*)(at + 4);
            if (string.IsNullOrEmpty(name) || number < 0 || number > 100000)
            {
                return false;
            }
            names[i] = number > 0 ? $"{name}_{number - 1}" : name;
            bip |= names[i].StartsWith("Bip01", StringComparison.OrdinalIgnoreCase);
        }
        return bip;
    }

    /// Checks a few parent links every Biped skeleton has.
    static bool LooksLikeBiped(string[] names, int[] parents)
    {
        var checks = 0;
        foreach (var (child, parent) in new[] { ("Bip01_L_Calf", "Bip01_L_Thigh"), ("Bip01_R_Calf", "Bip01_R_Thigh"), ("Bip01_L_Forearm", "Bip01_L_UpperArm"), ("Bip01_Spine1", "Bip01_Spine") })
        {
            var c = Array.IndexOf(names, child);
            var p = Array.IndexOf(names, parent);
            if (c < 0 || p < 0)
            {
                continue;
            }
            if (parents[c] != p)
            {
                return false;
            }
            checks++;
        }
        return checks >= 2;
    }

    static string Hex(byte* p, int words)
    {
        var parts = new string[words];
        for (var i = 0; i < words; i++)
        {
            parts[i] = (*(uint*)(p + i * 4)).ToString("X8");
        }
        return string.Join(" ", parts);
    }

    void Apply(RPawnPlayer? pawn, SkeletalMeshComponent? mesh)
    {
        var state = R32(Rep + RpState);
        var flags = R32(Rep + RpFlags);
        var key = (int)R32(Rep + RpToggleKey);
        if (key > 0)
        {
            _toggleKey = key;
        }
        var msgSeq = R32(Rep + RpMsgSeq);
        if (msgSeq != _lastMsgSeq)
        {
            _lastMsgSeq = msgSeq;
            Show(ReadText(Rep + RpMsg, 256));
        }
        s_hudSkating = (flags & Skating) != 0;
        s_score = s_hudSkating ? ReadText(Rep + RpScore, 128) : "";
        s_points = R32(Rep + RpHudPoints);
        s_total = R32(Rep + RpHudTotal);
        var resultSeq = R32(Rep + RpResultSeq);
        if (!s_resultSynced)
        {
            s_resultSynced = true;
            s_resultSeq = resultSeq;
        }
        else if (resultSeq != s_resultSeq)
        {
            s_resultSeq = resultSeq;
            var text = ReadText(Rep + RpResultText, 128);
            if (text.Length > 0)
            {
                s_resultKind = R32(Rep + RpResultKind);
                s_resultPoints = R32(Rep + RpResultPoints);
                s_resultText = text;
                s_resultAt = Environment.TickCount64;
                Debug.Log($"SkateCity: {text}: {s_resultPoints} pts ({s_resultKind})");
            }
        }
        if (state == StateFailed && _frames % 600 == 1)
        {
            Debug.Log("SkateCity: the skate engine failed to load - see SkateCity.log");
        }

        Step(19);
        PlaySounds(pawn);

        // Cast the rays the helper asked for; the answers go back next frame.
        Step(11);
        var batch = R32(Rep + RpBatch);
        if (pawn != null && batch != 0 && batch != _answeredBatch)
        {
            var count = (int)Math.Min(R32(Rep + RpRayCount), MaxRays);
            for (var i = 0; i < count; i++)
            {
                var ray = RepRays + i * 24;
                var hit = TraceSolid(pawn, RV(ray), RV(ray + 12), out var at);
                var o = ReqHits + i * 28;
                W32(o, hit ? 1u : 0u);
                WV(o + 4, at);
                WV(o + 16, Vector3.Zero);
            }
            _answeredBatch = batch;
            _answeredCount = (uint)count;
        }

        Step(12);
        var skating = (flags & Skating) != 0 && pawn != null && mesh != null;
        if (_skating && pawn != null && (pawn.Ptr != _skatePawn || pawn.bDeleteMe || pawn.bPendingDelete))
        {
            skating = false;
            pawn = null;
            _world++; // tells the helper to stop too
        }
        if (skating && !_skating)
        {
            StartSkating(pawn!, mesh!);
        }
        else if (!skating && _skating)
        {
            StopSkating();
        }
        Step(18);
        UpdateGlide(pawn);
        if (!_skating || pawn == null || mesh == null)
        {
            return;
        }

        Step(13);
        if ((flags & SetPawn) != 0)
        {
            _lastPawnLoc = RV(Rep + RpPawnLoc);
            pawn.SetLocation(_lastPawnLoc);
            pawn.SetRotation(RR(Rep + RpPawnRot));
        }
        Step(14);
        if ((flags & SetBones) != 0)
        {
            var count = (int)R32(Rep + RpBoneCount);
            var data = *(IntPtr*)(mesh.Ptr + SkelSpaceBases);
            var num = *(int*)(mesh.Ptr + SkelSpaceBases + 4);
            if (data != IntPtr.Zero && count > 0 && count <= MaxBones && num == count)
            {
                Buffer.MemoryCopy(_base + RepBones, (void*)data, num * 32, count * 32);
            }
            else if (_bonesMismatched++ % 300 == 0)
            {
                Debug.Log($"SkateCity: bone count mismatch ({count} posed, mesh has {num})");
            }
        }
        Step(15);
        CameraActive = (flags & SetCamera) != 0;
        if (CameraActive)
        {
            _cam ??= new DynamicCameraActor(pawn.Location, pawn.Rotation);
            if (_cam.IsValid)
            {
                _cam.SetLocation(RV(Rep + RpCamLoc));
                _cam.SetRotation(RR(Rep + RpCamRot));
                var pc = Game.GetPlayerController();
                if (pc.GetViewTarget()?.Ptr != _cam.Ptr)
                {
                    pc.SetViewTarget(_cam);
                }
            }
        }
        Step(16);
        if ((flags & SetBoard) != 0)
        {
            PlaceBoard((int)R32(Rep + RpBoardCount));
        }
    }

    /// Plays the sounds the helper asked for since last frame.
    void PlaySounds(RPawnPlayer? pawn)
    {
        var count = R32(Rep + RpSoundCount);
        if (!_soundSynced)
        {
            _soundSynced = true;
            _soundSeen = count;
            return;
        }
        if (count == _soundSeen)
        {
            return;
        }
        // The helper restarted: its count started again.
        if (count < _soundSeen)
        {
            _soundSeen = count;
            return;
        }
        var from = Math.Max(_soundSeen, count - SoundSlots);
        _soundSeen = count;
        if (pawn == null || !pawn.IsValid)
        {
            return;
        }
        if (!_soundsSearched)
        {
            _soundsSearched = true;
            FindSounds();
        }
        for (var n = from; n < count; n++)
        {
            var o = Rep + RpSounds + (int)(n % SoundSlots) * 16;
            var kind = (int)R32(o);
            if (kind <= 0 || kind >= _cues.Length || _cues[kind] is not { } cue || !cue.IsValid)
            {
                continue;
            }
            try
            {
                pawn.PlaySound(cue, true, false, false, RV(o + 4));
            }
            catch (Exception e)
            {
                Debug.Log($"SkateCity: playing a sound: {e.Message}");
                _cues[kind] = null;
            }
        }
    }

    /// The game's own sound cues for skating, found by name (and logged),
    /// unless SkateCity.ini names one (sound_pop, sound_land, sound_grind,
    /// sound_hit, sound_hard_hit, sound_break; "none" for silence).
    void FindSounds()
    {
        string[] keys = { "", "sound_pop", "sound_land", "sound_grind", "sound_hit", "sound_hard_hit", "sound_break" };
        (string Word, int Score)[][] words =
        {
            Array.Empty<(string, int)>(),
            new[] { ("skate", 5), ("board", 3), ("wood", 3), ("plank", 3), ("crate", 2), ("knock", 2), ("jump", 2), ("tap", 1) },
            new[] { ("land", 4), ("thud", 3), ("drop", 2), ("heavy", 1), ("foot", 1), ("impact", 1) },
            new[] { ("grind", 5), ("scrape", 4), ("slide", 3), ("rail", 2), ("metal", 2), ("pipe", 1) },
            new[] { ("punch", 3), ("hit", 3), ("body", 3), ("impact", 2), ("fall", 2), ("light", 1) },
            new[] { ("bodyfall", 5), ("slam", 4), ("crash", 3), ("impact", 3), ("heavy", 3), ("body", 2) },
            new[] { ("bonebreak", 6), ("bone", 5), ("crack", 4), ("snap", 4), ("break", 3) },
        };
        string[] avoid = { "music", "mus_", "vo_", "dialog", "dlg", "amb", "loop", "_lp", "ui_", "menu", "speech", "voice",
            "radio", "stinger", "cine", "vox", "riddler", "joker", "tv_", "alarm" };
        var overrides = ReadIni(keys);
        var best = new (int Score, SoundCue? Cue, string Path)[keys.Length];
        var seen = new List<string>[keys.Length];
        for (var k = 0; k < keys.Length; k++)
        {
            seen[k] = new List<string>();
        }
        try
        {
            foreach (var cue in Game.FindObjects<SoundCue>())
            {
                if (cue == null || !cue.IsValid)
                {
                    continue;
                }
                var path = cue.GetPathName();
                var lower = path.ToLowerInvariant();
                if (avoid.Any(lower.Contains))
                {
                    continue;
                }
                for (var k = 1; k < keys.Length; k++)
                {
                    var score = words[k].Where(x => lower.Contains(x.Word)).Sum(x => x.Score);
                    if (score <= 0)
                    {
                        continue;
                    }
                    if (seen[k].Count < 12)
                    {
                        seen[k].Add(path);
                    }
                    if (score > best[k].Score || (score == best[k].Score && path.Length < best[k].Path.Length))
                    {
                        best[k] = (score, cue, path);
                    }
                }
            }
        }
        catch (Exception e)
        {
            Debug.Log($"SkateCity: sound search failed: {e.Message}");
        }
        for (var k = 1; k < keys.Length; k++)
        {
            SoundCue? chosen = null;
            var from = "";
            if (overrides[k] is { } path)
            {
                if (path.Equals("none", StringComparison.OrdinalIgnoreCase))
                {
                    Debug.Log($"SkateCity: {keys[k]}: off (from the ini)");
                    continue;
                }
                try
                {
                    chosen = Game.FindObject<SoundCue>(path) ?? GameObject.DynamicLoadObject(path, SoundCue.StaticClass(), true) as SoundCue;
                    from = chosen != null ? $"{path} (from the ini)" : "";
                }
                catch
                {
                }
            }
            if (chosen == null && best[k].Cue != null)
            {
                chosen = best[k].Cue;
                from = best[k].Path;
            }
            _cues[k] = chosen;
            Debug.Log($"SkateCity: {keys[k]}: {(chosen != null ? from : "none found")}; candidates: {string.Join(" ; ", seen[k])}");
        }
    }

    /// Values for `keys` from SkateCity.ini next to the helper (null if unset).
    static string?[] ReadIni(string[] keys)
    {
        var values = new string?[keys.Length];
        try
        {
            var ini = Path.Combine(Path.GetDirectoryName(Environment.ProcessPath) ?? ".", "SkateCity", "SkateCity.ini");
            foreach (var line in File.Exists(ini) ? File.ReadAllLines(ini) : Array.Empty<string>())
            {
                var eq = line.IndexOf('=');
                if (eq < 0 || line.TrimStart().StartsWith(";"))
                {
                    continue;
                }
                var k = Array.IndexOf(keys, line[..eq].Trim().ToLowerInvariant());
                var value = line[(eq + 1)..].Trim();
                if (k >= 0 && value.Length > 0)
                {
                    values[k] = value;
                }
            }
        }
        catch (Exception e)
        {
            Debug.Log($"SkateCity: reading SkateCity.ini: {e.Message}");
        }
        return values;
    }

    /// LB in the air: the helper has stopped skating; open Batman's cape with
    /// the board's speed, and put him back on the board when he lands.
    void UpdateGlide(RPawnPlayer? pawn)
    {
        var seq = R32(Rep + RpGlideSeq);
        var endSeq = R32(Rep + RpGlideEndSeq);
        if (!_glideSynced)
        {
            _glideSynced = true;
            _glideSeq = seq;
            _glideEndSeq = endSeq;
        }
        var pc = Game.GetPlayerController();
        if (seq != _glideSeq)
        {
            _glideSeq = seq;
            _glideEndSeq = endSeq;
            if (pawn != null && pawn.IsValid && !_skating && pc != null)
            {
                pawn.SetPhysics(Actor.EPhysics.PHYS_Falling, false);
                pawn.Velocity = RV(Rep + RpGlideVel);
                // bForceGlide opens the cape; LockGlide then holds the glide
                // button for the player, so it steers like a normal glide.
                pc.bForceGlide = true;
                pc.LockGlide = true;
                for (var i = 0; i < 4 && pc.IsMoveInputIgnored(); i++)
                {
                    pc.IgnoreMoveInput(false);
                }
                for (var i = 0; i < 4 && pc.IsLookInputIgnored(); i++)
                {
                    pc.IgnoreLookInput(false);
                }
                _gliding = true;
                _capeOpenFrame = -1;
                _glideFrames = 0;
                _glideTries = 0;
                _landedFrames = 0;
                _glidePawn = pawn.Ptr;
                Debug.Log($"SkateCity: gliding off the board at {pawn.Velocity}");
            }
        }
        if (!_gliding)
        {
            return;
        }
        if (pawn == null || !pawn.IsValid || pawn.Ptr != _glidePawn || _skating || pc == null)
        {
            EndGlide(pc, false);
            return;
        }
        _glideFrames++;
        if (endSeq != _glideEndSeq)
        {
            _glideEndSeq = endSeq;
            pc.bForceGlide = false;
            pc.LockGlide = false;
            _glideTries = int.MaxValue;
        }
        // Once the cape is open, hand steering back to the player.
        if (_capeOpenFrame >= 0 && _glideFrames - _capeOpenFrame == 10)
        {
            pc.bForceGlide = false;
            Debug.Log($"SkateCity: glide steering handed back (move input ignored: {pc.IsMoveInputIgnored()})");
        }
        // Ask the game to open the cape until it does.
        if (pc.bForceGlide && _glideFrames >= 2 && _glideTries < 20)
        {
            _glideTries++;
            if (pc.CheckToCapeGlideFromFall(false))
            {
                Debug.Log($"SkateCity: cape open after {_glideTries} tries (physics {pawn.Physics})");
                _glideTries = int.MaxValue;
                _capeOpenFrame = _glideFrames;
            }
            else if (_glideTries == 20)
            {
                pc.ServerBeginCapeGlide();
                Debug.Log($"SkateCity: started the glide directly (physics {pawn.Physics})");
                _capeOpenFrame = _glideFrames;
            }
        }
        // Landed: back on the board.
        _landedFrames = pawn.Physics == Actor.EPhysics.PHYS_Walking ? _landedFrames + 1 : 0;
        if (_glideFrames > 15 && _landedFrames >= 5)
        {
            Debug.Log("SkateCity: landed from the glide; back on the board");
            EndGlide(pc, true);
        }
        else if (_glideFrames > 60 * 90)
        {
            EndGlide(pc, false);
        }
    }

    void EndGlide(RPlayerController? pc, bool skate)
    {
        _gliding = false;
        try
        {
            if (pc != null)
            {
                pc.bForceGlide = false;
                pc.LockGlide = false;
            }
        }
        catch
        {
        }
        if (skate)
        {
            _toggles++;
        }
    }

    /// Traces the world along a ray, including movable floors and rooftops,
    /// looking through invisible blocking volumes and characters.
    static bool TraceSolid(Actor tracer, Vector3 from, Vector3 to, out Vector3 at)
    {
        var dir = to - from;
        var length = dir.Length();
        if (length < 0.01f)
        {
            at = from;
            return false;
        }
        dir /= length;
        for (var k = 0; k < 8; k++)
        {
            var actor = tracer.Trace(out var hit, out _, to, from, true, Vector3.Zero, out var info, 0);
            if (actor == null)
            {
                break;
            }
            // Look through anything that isn't visible scenery: invisible
            // blockers, people, triggers, effects, cameras, pickups.
            if (!IsScenery(actor, info))
            {
                from = hit + dir * 2f;
                if (Vector3.Dot(to - from, dir) <= 0)
                {
                    break;
                }
                continue;
            }
            at = hit;
            return true;
        }
        at = Vector3.Zero;
        return false;
    }

    static bool IsScenery(Actor actor, in Actor.FTraceHitInfo info)
    {
        if (actor is WorldInfo)
        {
            return true; // level geometry
        }
        // Spawned meshes include our own board.
        if (actor is Volume || actor is Pawn || actor is DynamicSMActor_Spawnable)
        {
            return false;
        }
        var component = info.HitComponent;
        if (component != null && component.HiddenGame)
        {
            return false;
        }
        if (actor is StaticMeshCollectionActor)
        {
            return true;
        }
        if (actor.bHidden)
        {
            return false;
        }
        return actor.bWorldGeometry || actor is DynamicSMActor;
    }

    void StartSkating(RPawnPlayer pawn, SkeletalMeshComponent mesh)
    {
        _skating = true;
        _skatePawn = pawn.Ptr;
        _skateMesh = mesh.Ptr;
        _lastPawnLoc = pawn.Location;
        _saved = new EPhysicsSave
        {
            Physics = pawn.Physics,
            CollideActors = pawn.bCollideActors,
            BlockActors = pawn.bBlockActors,
            CollideWorld = pawn.bCollideWorld,
        };
        pawn.SetCollision(false, false, false);
        pawn.bCollideWorld = false;
        pawn.SetPhysics(Actor.EPhysics.PHYS_None, false);
        mesh.bNoSkeletonUpdate = true;
        try
        {
            var pc = Game.GetPlayerController();
            pc.IgnoreMoveInput(true);
            pc.IgnoreLookInput(true);
        }
        catch (Exception e)
        {
            Debug.Log($"SkateCity: could not block movement input: {e.Message}");
        }
        Debug.Log($"SkateCity: skating (physics was {_saved.Physics})");
    }

    void StopSkating()
    {
        CameraActive = false;
        s_score = "";
        if (!_skating)
        {
            return;
        }
        _skating = false;
        foreach (var part in _board)
        {
            if (part != null && part.IsValid)
            {
                part.SetHidden(true);
            }
        }
        try
        {
            // Only put back the character we changed: if he was removed (fell
            // out of the world, died, cutscene) the game has a fresh one.
            var pawn = Game.GetPlayerPawn();
            var same = pawn != null && pawn.IsValid && pawn.Ptr == _skatePawn && !pawn.bDeleteMe && !pawn.bPendingDelete;
            if (!same)
            {
                Debug.Log("SkateCity: the character changed while skating; not restoring the old one");
            }
            if (same)
            {
                var mesh = pawn!.Mesh;
                if (mesh != null && mesh.IsValid && mesh.Ptr == _skateMesh)
                {
                    mesh.bNoSkeletonUpdate = false;
                }
                pawn.bCollideWorld = _saved.CollideWorld;
                pawn.SetCollision(_saved.CollideActors, _saved.BlockActors, false);
                pawn.SetLocation(_lastPawnLoc + new Vector3(0, 0, 10));
                pawn.SetPhysics(Actor.EPhysics.PHYS_Falling, false);
            }
            var pc = Game.GetPlayerController();
            if (pawn != null && pawn.IsValid && !pawn.bDeleteMe)
            {
                pc.SetViewTarget(pawn);
            }
            pc.IgnoreMoveInput(false);
            pc.IgnoreLookInput(false);
        }
        catch (Exception e)
        {
            Debug.Log($"SkateCity: restoring Batman: {e.Message}");
        }
        Debug.Log("SkateCity: stopped skating");
    }

    /// The board: deck, nose and tail (grip), two trucks (metal) and four
    /// wheels, from the engine's box and sphere meshes and materials found in
    /// the game. Parts come in that order from the helper.
    static int PartKind(int i) => i < 3 ? 0 : i < 5 ? 1 : 2;

    static readonly string[] BoxMeshes = { "EngineMeshes.Cube", "EditorMeshes.TexPropCube", "EngineMeshes.ParticleCube", "EditorMeshes.EditorCube" };
    static readonly string[] RoundMeshes = { "EngineMeshes.Sphere", "EditorMeshes.TexPropSphere", "EngineMeshes.ParticleSphere", "EditorMeshes.EditorSphere" };

    static StaticMesh? LoadMesh(string[] paths)
    {
        foreach (var path in paths)
        {
            try
            {
                var mesh = Game.FindObject<StaticMesh>(path) ?? GameObject.DynamicLoadObject(path, StaticMesh.StaticClass(), true) as StaticMesh;
                if (mesh != null)
                {
                    Debug.Log($"SkateCity: board uses {path}");
                    return mesh;
                }
            }
            catch
            {
            }
        }
        return null;
    }

    /// Materials for the grip, the trucks and the wheels: from SkateCity.ini
    /// if set there (board_grip_material, board_metal_material,
    /// board_wheel_material), else the best-named ones the game has loaded.
    void FindBoardMaterials()
    {
        string[] keys = { "board_grip_material", "board_metal_material", "board_wheel_material" };
        (string Word, int Score)[][] words =
        {
            new[] { ("griptape", 6), ("grip", 5), ("rubber", 4), ("asphalt", 3), ("tarmac", 3), ("black", 2), ("felt", 2), ("tar_", 2) },
            new[] { ("chrome", 5), ("steel", 4), ("alumin", 4), ("metal", 3), ("iron", 3), ("pipe", 1) },
            new[] { ("urethane", 6), ("wheel", 4), ("plastic", 4), ("white", 3), ("ceramic", 3), ("porcelain", 3), ("plaster", 2) },
        };
        string[] avoid = { "decal", "glass", "water", "fx", "particle", "emiss", "light", "glow", "sky", "blood", "hud", "ui_", "font", "trans",
            "fog", "fire", "smoke", "spark", "hair", "eye", "skin", "cape", "face", "mask", "lens", "post", "refract", "holo", "shadow",
            "debug", "editor", "wire", "screen", "tire", "tyre" };
        var overrides = new string?[3];
        try
        {
            var ini = Path.Combine(Path.GetDirectoryName(Environment.ProcessPath) ?? ".", "SkateCity", "SkateCity.ini");
            foreach (var line in File.Exists(ini) ? File.ReadAllLines(ini) : Array.Empty<string>())
            {
                var eq = line.IndexOf('=');
                if (eq < 0 || line.TrimStart().StartsWith(";"))
                {
                    continue;
                }
                var k = Array.IndexOf(keys, line[..eq].Trim().ToLowerInvariant());
                var value = line[(eq + 1)..].Trim();
                if (k >= 0 && value.Length > 0)
                {
                    overrides[k] = value;
                }
            }
        }
        catch (Exception e)
        {
            Debug.Log($"SkateCity: reading board materials from the ini: {e.Message}");
        }
        var best = new (int Score, MaterialInterface? Material, string Path)[3];
        var seen = new List<string>[] { new(), new(), new() };
        try
        {
            foreach (var material in Game.FindObjects<MaterialInterface>())
            {
                if (material == null || !material.IsValid)
                {
                    continue;
                }
                var path = material.GetPathName();
                var lower = path.ToLowerInvariant();
                if (avoid.Any(lower.Contains))
                {
                    continue;
                }
                for (var k = 0; k < 3; k++)
                {
                    var score = words[k].Where(w => lower.Contains(w.Word)).Sum(w => w.Score);
                    if (score <= 0)
                    {
                        continue;
                    }
                    if (seen[k].Count < 12)
                    {
                        seen[k].Add(path);
                    }
                    if (score > best[k].Score || (score == best[k].Score && path.Length < best[k].Path.Length))
                    {
                        best[k] = (score, material, path);
                    }
                }
            }
        }
        catch (Exception e)
        {
            Debug.Log($"SkateCity: material search failed: {e.Message}");
        }
        for (var k = 0; k < 3; k++)
        {
            MaterialInterface? chosen = null;
            var from = "";
            if (overrides[k] is { } path)
            {
                try
                {
                    chosen = Game.FindObject<MaterialInterface>(path) ?? GameObject.DynamicLoadObject(path, MaterialInterface.StaticClass(), true) as MaterialInterface;
                    from = chosen != null ? $"{path} (from the ini)" : "";
                }
                catch
                {
                }
                if (chosen == null)
                {
                    Debug.Log($"SkateCity: {keys[k]} {path} not found; picking one");
                }
            }
            if (chosen == null && best[k].Material != null)
            {
                chosen = best[k].Material;
                from = best[k].Path;
            }
            _boardMaterials[k] = chosen;
            Debug.Log($"SkateCity: {keys[k]}: {(chosen != null ? from : "none (default look)")}; candidates: {string.Join(" ; ", seen[k])}");
        }
    }

    void PlaceBoard(int count)
    {
        if (!_cubeSearched)
        {
            _cubeSearched = true;
            _cube = LoadMesh(BoxMeshes);
            _round = LoadMesh(RoundMeshes);
            if (_cube == null)
            {
                Debug.Log("SkateCity: no box mesh found; skating without a visible board");
            }
            FindBoardMaterials();
        }
        if (_cube == null)
        {
            return;
        }
        count = Math.Min(count, _board.Length);
        for (var i = 0; i < count; i++)
        {
            var o = i < 8 ? Rep + RpBoard + i * 36 : Rep + RpBoardMore + (i - 8) * 36;
            var loc = RV(o);
            var rot = RR(o + 12);
            var size = RV(o + 24);
            var kind = PartKind(i);
            var round = kind == 2 && _round != null;
            var part = _board[i];
            if (part == null || !part.IsValid)
            {
                part = new DynamicSMActor_Spawnable(loc, default);
                part.SetCollision(false, false, false);
                part.StaticMeshComponent.SetStaticMesh(round ? _round! : _cube, false);
                if (_boardMaterials[kind] is { } material)
                {
                    part.StaticMeshComponent.SetMaterial(0, material);
                }
                _board[i] = part;
            }
            ref var extent = ref (round ? ref _roundExtent : ref _cubeExtent);
            if (extent.X < 0.01f)
            {
                // Measure the mesh unrotated and unscaled: Bounds is world-aligned.
                part.SetRotation(default);
                part.StaticMeshComponent.SetScale3D(Vector3.One);
                var measured = part.StaticMeshComponent.Bounds.BoxExtent;
                if (measured.X > 0.01f && measured.Y > 0.01f && measured.Z > 0.01f)
                {
                    extent = measured;
                    Debug.Log($"SkateCity: {(round ? "wheel" : "box")} mesh half-size {measured}");
                }
            }
            if (extent.X >= 0.01f)
            {
                part.StaticMeshComponent.SetScale3D(new Vector3(size.X / (2 * extent.X), size.Y / (2 * extent.Y), size.Z / (2 * extent.Z)));
            }
            part.SetLocation(loc);
            part.SetRotation(rot);
            part.SetHidden(false);
        }
        for (var i = count; i < _board.Length; i++)
        {
            if (_board[i] is { IsValid: true } extra)
            {
                extra.SetHidden(true);
            }
        }
    }

    static void Show(string text)
    {
        if (string.IsNullOrWhiteSpace(text))
        {
            return;
        }
        s_message = text;
        s_messageAt = Environment.TickCount64;
        Debug.Log(text);
    }

    public static void DrawHud(RHUD hud)
    {
        var now = Environment.TickCount64;
        var showMessage = s_message.Length > 0 && now - s_messageAt < 5000;
        var resultAge = now - s_resultAt;
        var shown = s_resultKind == ResultSlam ? SlamShown : ResultShown;
        var showResult = s_resultText.Length > 0 && resultAge < shown + ResultFade;
        if (!showMessage && !s_hudSkating && !showResult)
        {
            return;
        }
        var canvas = hud.Canvas;
        if (canvas == null || !canvas.IsValid)
        {
            return;
        }
        var w = canvas.ClipX;
        var h = canvas.ClipY;
        var ui = h / 1080f;
        var medium = _Engine.GetMediumFont();
        var large = _Engine.GetLargeFont() ?? medium;

        // Session score, top right.
        if (s_hudSkating && s_total > 0)
        {
            canvas.Font = medium;
            var text = $"SCORE  {Thousands(s_total)}";
            var tw = Measure(canvas, text, 0.9f * ui, out var sh);
            DrawLogo(canvas, w - tw - w * 0.04f - sh * 1.2f, h * 0.17f + sh / 2, sh * 1.3f, 220);
            canvas.Font = medium;
            Shadowed(canvas, text, w - tw - w * 0.04f, h * 0.17f, 0.9f * ui, 235, 235, 235, 230);
        }

        // The line being skated: big points, the trick chain under them.
        if (s_score.Length > 0)
        {
            canvas.Font = large;
            var big = Thousands(s_points);
            var bw = Measure(canvas, big, 1.1f * ui, out var bh);
            var y = h * 0.74f;
            DrawLogo(canvas, w / 2, y + bh * 0.75f, h * 0.17f, 150);
            canvas.Font = large;
            Shadowed(canvas, big, (w - bw) / 2, y, 1.1f * ui, 255, 214, 80, 255);
            canvas.Font = medium;
            var cw = Measure(canvas, s_score, 0.85f * ui, out _);
            var scale = cw > w * 0.8f ? 0.85f * ui * (w * 0.8f / cw) : 0.85f * ui;
            cw = Measure(canvas, s_score, scale, out _);
            Shadowed(canvas, s_score, (w - cw) / 2, y + bh + 6 * ui, scale, 255, 255, 255, 240);
        }
        // The line just finished: how it went, rising and fading out.
        else if (showResult)
        {
            var alpha = resultAge < shown ? 1f : 1f - (resultAge - shown) / (float)ResultFade;
            var a = (byte)Math.Clamp(255 * alpha, 0, 255);
            var rise = Math.Min(resultAge, 250) / 250f;
            var lines = s_resultText.Split('|');
            var y = h * (lines.Length > 1 ? 0.70f : 0.76f) - h * 0.03f * rise;
            var (label, r, g, b) = s_resultKind switch
            {
                ResultClean => ("CLEAN!", (byte)90, (byte)230, (byte)120),
                ResultSketchy => ("SKETCHY", (byte)255, (byte)185, (byte)60),
                ResultBailed => ("BAILED", (byte)255, (byte)80, (byte)70),
                ResultSlam => ("SLAM!", (byte)255, (byte)110, (byte)40),
                _ => ("LANDED", (byte)235, (byte)235, (byte)235),
            };
            var headline = s_resultKind == ResultBailed ? label : $"+{Thousands(s_resultPoints)}   {label}";
            canvas.Font = large;
            var hw = Measure(canvas, headline, 1.1f * ui, out var hh);
            // Each line of detail under the headline, shrunk to fit if long.
            canvas.Font = medium;
            var widths = new float[lines.Length];
            var scales = new float[lines.Length];
            float textHeight = 0, widest = hw;
            for (var i = 0; i < lines.Length; i++)
            {
                var size = i == 0 ? 0.8f * ui : 0.65f * ui;
                var lw = Measure(canvas, lines[i], size, out _);
                scales[i] = lw > w * 0.8f ? size * (w * 0.8f / lw) : size;
                widths[i] = Measure(canvas, lines[i], scales[i], out var lh);
                textHeight += lh + 4 * ui;
                widest = Math.Max(widest, widths[i]);
            }
            Panel(canvas, (w - widest) / 2 - 24 * ui, y - 10 * ui, widest + 48 * ui, hh + textHeight + 22 * ui, (byte)(a * 0.45f));
            DrawLogo(canvas, w / 2, y + (hh + textHeight) / 2, h * 0.17f, (byte)(a * 0.6f));
            canvas.Font = large;
            Shadowed(canvas, headline, (w - hw) / 2, y, 1.1f * ui, r, g, b, a);
            canvas.Font = medium;
            var ly = y + hh + 6 * ui;
            for (var i = 0; i < lines.Length; i++)
            {
                var best = lines[i].StartsWith("NEW BEST");
                var (lr, lg, lb) = best ? ((byte)255, (byte)214, (byte)80) : i == 0 ? ((byte)255, (byte)255, (byte)255) : ((byte)200, (byte)200, (byte)200);
                Shadowed(canvas, lines[i], (w - widths[i]) / 2, ly, scales[i], lr, lg, lb, a);
                Measure(canvas, lines[i], scales[i], out var lh);
                ly += lh + 4 * ui;
            }
        }

        // Messages from SkateCity itself, lower left.
        if (showMessage)
        {
            canvas.Font = medium;
            Shadowed(canvas, s_message, w * 0.04f, h * 0.9f, 0.8f * ui, 255, 255, 255, 230);
        }
    }

    const int ResultShown = 2200, SlamShown = 4500, ResultFade = 600;

    static Texture2D? s_logo;
    static bool s_logoSearched;

    /// The bat symbol from the game's own textures, found by name once.
    static Texture2D? Logo()
    {
        if (s_logoSearched)
        {
            return s_logo;
        }
        s_logoSearched = true;
        try
        {
            var found = new List<(int Score, Texture2D Texture, string Path)>();
            foreach (var texture in Game.FindObjects<Texture2D>())
            {
                if (texture == null || !texture.IsValid)
                {
                    continue;
                }
                var path = texture.GetPathName();
                var lower = path.ToLowerInvariant();
                if (!lower.Contains("bat"))
                {
                    continue;
                }
                var score = 0;
                if (lower.Contains("logo")) score += 4;
                if (lower.Contains("symbol")) score += 4;
                if (lower.Contains("emblem") || lower.Contains("insignia") || lower.Contains("signal")) score += 3;
                if (lower.Contains("hud") || lower.Contains("icon") || lower.Contains("ui")) score += 1;
                if (lower.EndsWith("_n") || lower.Contains("normal") || lower.Contains("spec") || lower.Contains("mask") || lower.Contains("_s.")) score -= 3;
                if (texture.SizeX < 64 || texture.SizeY < 32) score -= 2;
                if (score > 0)
                {
                    found.Add((score, texture, path));
                }
            }
            found.Sort((a, b) => b.Score.CompareTo(a.Score));
            Debug.Log($"SkateCity: bat symbol candidates ({found.Count}): " +
                string.Join(" ; ", found.Take(25).Select(f => $"{f.Path} {f.Texture.SizeX}x{f.Texture.SizeY} [{f.Score}]")));
            s_logo = found.Count > 0 ? found[0].Texture : null;
            Debug.Log(s_logo != null ? $"SkateCity: using {found[0].Path} behind the score" : "SkateCity: no bat symbol texture found");
        }
        catch (Exception e)
        {
            Debug.Log($"SkateCity: bat symbol search failed: {e.Message}");
        }
        return s_logo;
    }

    /// Draws the bat symbol centred on (cx, cy), `height` tall.
    static void DrawLogo(Canvas canvas, float cx, float cy, float height, byte alpha)
    {
        var logo = Logo();
        if (logo == null || !logo.IsValid || logo.SizeY <= 0)
        {
            return;
        }
        var width = height * logo.SizeX / logo.SizeY;
        canvas.SetDrawColor(255, 255, 255, alpha);
        canvas.SetPos(cx - width / 2, cy - height / 2, 0);
        canvas.DrawRect(width, height, logo);
    }

    static float Measure(Canvas canvas, string text, float scale, out float height)
    {
        canvas.TextSize(text, out var xl, out var yl);
        height = yl * scale;
        return xl * scale;
    }

    static void Shadowed(Canvas canvas, string text, float x, float y, float scale, byte r, byte g, byte b, byte a)
    {
        canvas.SetDrawColor(0, 0, 0, (byte)(a * 0.85f));
        canvas.SetPos(x + 2, y + 2, 0);
        canvas.DrawText(text, false, scale, scale, out _);
        canvas.SetDrawColor(r, g, b, a);
        canvas.SetPos(x, y, 0);
        canvas.DrawText(text, false, scale, scale, out _);
    }

    static void Panel(Canvas canvas, float x, float y, float width, float height, byte alpha)
    {
        var texture = canvas.DefaultTexture;
        if (texture == null || !texture.IsValid)
        {
            return;
        }
        canvas.SetDrawColor(0, 0, 0, alpha);
        canvas.SetPos(x, y, 0);
        canvas.DrawRect(width, height, texture);
    }

    static string Thousands(uint n) => n.ToString("N0", System.Globalization.CultureInfo.InvariantCulture);
}
