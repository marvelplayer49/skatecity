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
    public override void OnEnterGame() => SkateLink.EnterGameAll();

    public override void OnTick() => SkateLink.TickAll();

    // The keyboard belongs to player 1.
    public override void OnKeyDown(Keys key) => SkateLink.ForPlayer(0).KeyDown(key);

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
            SkateLink.DrawHudFor(self);
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
    // One link (and one helper, skate engine, board, camera and HUD) per local
    // player, for split-screen co-op. Player 1 is index 0.
    const int MaxPlayers = 4;
    static readonly SkateLink?[] s_links = new SkateLink?[MaxPlayers];
    /// The link being ticked, for the crash breadcrumbs.
    static SkateLink? s_current;
    readonly int _player;

    SkateLink(int player)
    {
        _player = player;
    }

    public static SkateLink ForPlayer(int player) => s_links[player] ??= new SkateLink(player);

    /// How many local players there are (1 without split-screen).
    static int PlayerCount()
    {
        try
        {
            return Math.Clamp(Game.GetEngine().GamePlayers.Count, 1, MaxPlayers);
        }
        catch
        {
            return 1;
        }
    }

    /// This link's player controller and pawn, straight from the engine's
    /// list of local players (null if that player isn't there).
    RPlayerController? Controller()
    {
        try
        {
            var players = Game.GetEngine().GamePlayers;
            return _player < players.Count ? players[_player]?.Actor as RPlayerController : null;
        }
        catch
        {
            return null;
        }
    }

    RPawnPlayer? PlayerPawn()
    {
        try
        {
            return Controller()?.CombatPawn as RPawnPlayer;
        }
        catch
        {
            return null;
        }
    }

    // The clock of the last frame ticked: with some mods loaded (the
    // Split-Screen Mod) scripts get ticked twice a frame, which would run
    // skating at double speed. One tick per frame, whatever calls it.
    static float s_lastFrameTime = float.NaN;
    static bool s_extraTickLogged;

    public static void TickAll()
    {
        try
        {
            var now = Game.GetWorldInfo().RealTimeSeconds;
            if (now == s_lastFrameTime)
            {
                if (!s_extraTickLogged)
                {
                    s_extraTickLogged = true;
                    Debug.Log("SkateCity: the game ticked scripts more than once in a frame; ignoring the extra ticks");
                }
                return;
            }
            s_lastFrameTime = now;
        }
        catch
        {
        }
        var count = PlayerCount();
        for (var i = 0; i < MaxPlayers; i++)
        {
            // Players who left still get ticked, so their skating stops cleanly.
            if (i >= count && s_links[i] == null)
            {
                continue;
            }
            var link = ForPlayer(i);
            s_current = link;
            link.Tick();
        }
    }

    public static void EnterGameAll()
    {
        foreach (var link in s_links)
        {
            link?.EnterGame();
        }
        ForPlayer(0);
    }

    /// Draws the HUD of whichever player this HUD belongs to.
    public static void DrawHudFor(RHUD hud)
    {
        var owner = hud.PlayerOwner;
        foreach (var link in s_links)
        {
            if (link != null && owner != null && link.Controller() is { } pc && pc.Ptr == owner.Ptr)
            {
                link.DrawHud(hud);
                return;
            }
        }
        if (PlayerCount() == 1)
        {
            s_links[0]?.DrawHud(hud);
        }
    }

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
        RpSoundCount = 1004, RpSounds = 1012, SoundSlots = 8,
        RpChalKind = 1140, RpChalDone = 1144, RpChalLoc = 1148, RpChalText = 1160;
    const uint ResultClean = 1, ResultSketchy = 3, ResultBailed = 4, ResultSlam = 5, ResultChallenge = 6;
    const uint PawnValid = 1, SkeletonValid = 2;
    const uint SetPawn = 1, SetCamera = 2, SetBones = 4, SetBoard = 8, Skating = 16;
    const uint StateFailed = 4;

    // Offsets into engine objects (from BmSDK's generated classes).
    const int ComponentLocalToWorld = 112;
    const int SkelSpaceBases = 600, SkelLocalAtoms = 612;

    bool CameraActive;
    string s_message = "";
    long s_messageAt;
    string s_score = "";
    // The trick display.
    bool s_hudSkating, s_resultSynced;
    uint s_points, s_total, s_resultSeq, s_resultKind, s_resultPoints;
    string s_resultText = "";
    long s_resultAt = -100000;
    // The active challenge: 1 grind, 2 drop, 3 line (0 none).
    uint s_chalKind, s_chalDone;
    Vector3 s_chalLoc;
    string s_chalText = "";

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
    // Wwise event for each kind (1 pop, 2 land, 3 grind, 4 hit, 5 hard hit,
    // 6 break). Arkham City plays all its sound through Wwise.
    uint _soundSeen;
    bool _soundSynced, _soundsSearched;
    int _parkErrors, _parkHitLogs;
    readonly AkEvent?[] _cues = new AkEvent?[7];
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
        for (var i = 0; i < MaxPlayers; i++)
        {
            var link = s_links[i];
            s_links[i] = null;
            link?.Close();
        }
        s_current = null;
    }

    void Close()
    {
        var link = this;
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
        if (_player == 0)
        {
            Park.NewWorld();
        }
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
            var name = $"Local\\SkateCity_{pid}{(_player == 0 ? "" : $"_p{_player + 1}")}";
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

            // Player 1's link starts first: clear out helpers left from a crash.
            if (_player == 0)
            {
                foreach (var old in System.Diagnostics.Process.GetProcessesByName("SkateCity"))
                {
                    try { old.Kill(); } catch { }
                }
            }
            var binaries = Path.GetDirectoryName(Environment.ProcessPath) ?? ".";
            var exe = Path.Combine(binaries, "SkateCity", "SkateCity.exe");
            if (!File.Exists(exe))
            {
                Debug.LogError($"SkateCity: {exe} is missing - run build.bat in the SkateCity folder");
                _startFailed = true;
                return;
            }
            var args = _player == 0 ? pid.ToString() : $"{pid} {_player}";
            _helper = System.Diagnostics.Process.Start(new System.Diagnostics.ProcessStartInfo(exe, args)
            {
                UseShellExecute = false,
                CreateNoWindow = true,
                WorkingDirectory = Path.GetDirectoryName(exe),
            });
            Debug.Log($"SkateCity: player {_player + 1}'s helper started (process {_helper?.Id}); loading Skate 3 data");
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
        var link = s_current;
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
            pawn = PlayerPawn();
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
        if (_player == 0)
        {
            try
            {
                Park.Tick(valid ? pawn : null, _skating, Park.GameFocused());
            }
            catch (Exception e)
            {
                if (_parkErrors++ % 300 == 0)
                {
                    Debug.Log($"SkateCity: park: {e}");
                }
            }
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
        s_chalKind = s_hudSkating ? R32(Rep + RpChalKind) : 0;
        s_chalDone = R32(Rep + RpChalDone);
        s_chalLoc = RV(Rep + RpChalLoc);
        s_chalText = s_chalKind != 0 ? ReadText(Rep + RpChalText, 96) : "";
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
            if (Park.Hits > 0 && _parkHitLogs < 12)
            {
                _parkHitLogs++;
                Debug.Log($"SkateCity: park: {Park.Hits} of the scan's rays hit park pieces");
            }
            Park.Hits = 0;
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
                var pc = Controller();
                if (pc != null && pc.GetViewTarget()?.Ptr != _cam.Ptr)
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

    /// Sound kinds from the helper, by WAV name: pop, land, grind, hit,
    /// hard hit (slams), bone break.
    static readonly string[] s_wavNames = { "", "pop", "land", "grind", "hit", "hardhit", "break" };

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
            // Our own WAVs first (Binaries\Win32\SkateCity\sounds).
            if (kind > 0 && kind < s_wavNames.Length && SkateSfx.Has(s_wavNames[kind]))
            {
                SkateSfx.Play(s_wavNames[kind]);
                continue;
            }
            if (kind <= 0 || kind >= _cues.Length || _cues[kind] is not { } cue || !cue.IsValid)
            {
                continue;
            }
            try
            {
                pawn.StartAuxAudioEvent(cue, RV(o + 4));
            }
            catch (Exception e)
            {
                Debug.Log($"SkateCity: playing a sound: {e.Message}");
                _cues[kind] = null;
            }
        }
    }

    /// The game's own Wwise sound events for skating, found by name (and logged),
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
            "radio", "stinger", "cine", "vox", "riddler", "joker", "tv_", "alarm", "stop_", "_stop", "pause", "resume", "mute" };
        var overrides = ReadIni(keys);
        var best = new (int Score, AkEvent? Cue, string Path)[keys.Length];
        var total = 0;
        var sample = new List<string>();
        var seen = new List<string>[keys.Length];
        for (var k = 0; k < keys.Length; k++)
        {
            seen[k] = new List<string>();
        }
        try
        {
            foreach (var cue in Game.FindObjects<AkEvent>())
            {
                if (cue == null || !cue.IsValid)
                {
                    continue;
                }
                total++;
                var path = cue.GetPathName();
                var lower = path.ToLowerInvariant();
                if (sample.Count < 60 && (lower.Contains("land") || lower.Contains("impact") || lower.Contains("body") || lower.Contains("bone")
                    || lower.Contains("metal") || lower.Contains("jump") || lower.Contains("wood") || lower.Contains("scrape")))
                {
                    sample.Add(path);
                }
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
            AkEvent? chosen = null;
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
                    chosen = Game.FindObject<AkEvent>(path) ?? GameObject.DynamicLoadObject(path, AkEvent.StaticClass(), true) as AkEvent;
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
        Debug.Log($"SkateCity: {total} sound events loaded; some by name: {string.Join(" ; ", sample)}");
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
        var pc = Controller();
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
        // The park, along the whole ray (before anything below moves `from`).
        var onPark = Park.Raycast(from, to, out var parkAt);
        if (onPark)
        {
            Park.Hits++;
        }
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
            // A park piece in front of the scenery?
            if (onPark && Vector3.Dot(parkAt - hit, dir) < 0)
            {
                hit = parkAt;
            }
            at = hit;
            return true;
        }
        // Nothing but maybe the park.
        at = onPark ? parkAt : Vector3.Zero;
        return onPark;
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
            var pc = Controller();
            pc?.IgnoreMoveInput(true);
            pc?.IgnoreLookInput(true);
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
            var pawn = PlayerPawn();
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
            var pc = Controller();
            if (pc != null && pawn != null && pawn.IsValid && !pawn.bDeleteMe)
            {
                pc.SetViewTarget(pawn);
            }
            pc?.IgnoreMoveInput(false);
            pc?.IgnoreLookInput(false);
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

    void Show(string text)
    {
        if (string.IsNullOrWhiteSpace(text))
        {
            return;
        }
        s_message = text;
        s_messageAt = Environment.TickCount64;
        Debug.Log(text);
    }

    void DrawHud(RHUD hud)
    {
        if (_player == 0 && hud.Canvas is { IsValid: true } parkCanvas)
        {
            try
            {
                Park.DrawHud(parkCanvas, parkCanvas.ClipX, parkCanvas.ClipY, parkCanvas.ClipY / 1080f, _Engine.GetMediumFont());
            }
            catch (Exception e)
            {
                if (_parkErrors++ % 300 == 0)
                {
                    Debug.Log($"SkateCity: park HUD: {e.Message}");
                }
            }
        }
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
                ResultChallenge => ("CHALLENGE!", (byte)110, (byte)200, (byte)255),
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

        if (s_chalKind != 0)
        {
            DrawChallenge(canvas, hud, w, h, ui, medium);
        }

        // Messages from SkateCity itself, lower left.
        if (showMessage)
        {
            canvas.Font = medium;
            Shadowed(canvas, s_message, w * 0.04f, h * 0.9f, 0.8f * ui, 255, 255, 255, 230);
        }
    }

    const int ResultShown = 2200, SlamShown = 4500, ResultFade = 600;

    /// The challenge: a panel at the top left, and a marker over the spot
    /// (or at the screen's edge pointing to it when it's out of view).
    void DrawChallenge(Canvas canvas, RHUD hud, float w, float h, float ui, Font medium)
    {
        var (name, r, g, b) = s_chalKind switch
        {
            1 => ("GRIND", (byte)110, (byte)200, (byte)255),
            2 => ("DROP", (byte)255, (byte)150, (byte)60),
            _ => ("LINE", (byte)140, (byte)230, (byte)120),
        };
        canvas.Font = medium;
        var x = w * 0.04f;
        var y = h * 0.12f;
        var title = $"CHALLENGE  ({s_chalDone} done)";
        var tw = Measure(canvas, title, 0.7f * ui, out var th);
        var lw = Measure(canvas, s_chalText, 0.75f * ui, out var lh);
        Panel(canvas, x - 12 * ui, y - 8 * ui, Math.Max(tw, lw) + 24 * ui, th + lh + 22 * ui, 110);
        Shadowed(canvas, title, x, y, 0.7f * ui, r, g, b, 255);
        Shadowed(canvas, s_chalText, x, y + th + 6 * ui, 0.75f * ui, 255, 255, 255, 245);

        // Where the spot is relative to the camera.
        var pc = Controller();
        if (pc == null)
        {
            return;
        }
        pc.GetPlayerViewPoint(out var eye, out var view);
        var forward = view.ToDirection();
        var to = s_chalLoc - eye;
        var label = $"[ {name} ]";
        var mw = Measure(canvas, label, 0.8f * ui, out var mh);
        if (Vector3.Dot(to, forward) > 0)
        {
            var screen = canvas.Project(s_chalLoc);
            if (screen.X >= 0 && screen.X <= w && screen.Y >= 0 && screen.Y <= h)
            {
                Shadowed(canvas, label, screen.X - mw / 2, screen.Y - mh, 0.8f * ui, r, g, b, 255);
                Shadowed(canvas, "V", screen.X - Measure(canvas, "V", 0.8f * ui, out _) / 2, screen.Y, 0.8f * ui, r, g, b, 255);
                return;
            }
        }
        // Out of view: point left or right. Unreal's axes are left-handed
        // (x forward, y right, z up), so forward x up points left.
        var left = Vector3.Cross(forward, Vector3.UnitZ);
        var onRight = Vector3.Dot(to, left) < 0;
        var arrow = onRight ? $"{label} >>" : $"<< {label}";
        var aw = Measure(canvas, arrow, 0.8f * ui, out _);
        Shadowed(canvas, arrow, onRight ? w - aw - w * 0.03f : w * 0.03f, h * 0.45f, 0.8f * ui, r, g, b, 255);
    }

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

/// Skating sounds: WAV files in Binaries\Win32\SkateCity\sounds (16-bit PCM),
/// played straight through Windows' wave output, several at once.
/// `Play("thwip")` picks one of thwip1.wav, thwip2.wav, ... at random.
/// Missing files are just skipped.
static unsafe class SkateSfx
{
    [StructLayout(LayoutKind.Sequential)]
    struct WaveFormat
    {
        public ushort Tag, Channels;
        public uint Rate, BytesPerSecond;
        public ushort BlockAlign, Bits, Extra;
    }

    [StructLayout(LayoutKind.Sequential)]
    struct WaveHdr
    {
        public IntPtr Data;
        public uint Length, Recorded;
        public IntPtr User;
        public uint Flags, Loops;
        public IntPtr Next, Reserved;
    }

    [DllImport("winmm.dll")]
    static extern int waveOutOpen(out IntPtr handle, uint device, ref WaveFormat format, IntPtr callback, IntPtr instance, uint flags);
    [DllImport("winmm.dll")]
    static extern int waveOutPrepareHeader(IntPtr handle, WaveHdr* header, int size);
    [DllImport("winmm.dll")]
    static extern int waveOutUnprepareHeader(IntPtr handle, WaveHdr* header, int size);
    [DllImport("winmm.dll")]
    static extern int waveOutWrite(IntPtr handle, WaveHdr* header, int size);
    [DllImport("winmm.dll")]
    static extern int waveOutReset(IntPtr handle);
    [DllImport("winmm.dll")]
    static extern int waveOutClose(IntPtr handle);

    const uint WaveMapper = 0xFFFFFFFF;
    const uint WhdrDone = 1;
    const int MaxVoices = 10;

    sealed class Clip
    {
        public WaveFormat Format;
        public IntPtr Data;
        public int Length;
    }

    sealed class Voice
    {
        public IntPtr Handle;
        public WaveHdr* Header;
    }

    static readonly Dictionary<string, List<Clip>> s_clips = new();
    static readonly List<Voice> s_voices = new();
    static readonly Random s_rng = new();
    static bool s_scanned;
    static int s_logs;

    public static string Folder => Path.GetFullPath(Path.Combine(Path.GetDirectoryName(Environment.ProcessPath) ?? ".", "SkateCity", "sounds"));

    /// Loudness, 0..1.
    public static float Volume = 0.9f;

    static void Log(string text)
    {
        if (s_logs++ < 12)
        {
            Debug.Log("SkateCity: " + text);
        }
    }

    static Clip? Load(string file)
    {
        var bytes = File.ReadAllBytes(file);
        if (bytes.Length < 12 || bytes[0] != 'R' || bytes[8] != 'W')
        {
            return null;
        }
        WaveFormat? format = null;
        var o = 12;
        while (o + 8 <= bytes.Length)
        {
            var id = System.Text.Encoding.ASCII.GetString(bytes, o, 4);
            var size = BitConverter.ToInt32(bytes, o + 4);
            var body = o + 8;
            if (id == "fmt ")
            {
                format = new WaveFormat
                {
                    Tag = BitConverter.ToUInt16(bytes, body),
                    Channels = BitConverter.ToUInt16(bytes, body + 2),
                    Rate = BitConverter.ToUInt32(bytes, body + 4),
                    BytesPerSecond = BitConverter.ToUInt32(bytes, body + 8),
                    BlockAlign = BitConverter.ToUInt16(bytes, body + 12),
                    Bits = BitConverter.ToUInt16(bytes, body + 14),
                };
            }
            else if (id == "data" && format is { } f && f.Tag == 1 && f.Bits == 16)
            {
                size = Math.Min(size, bytes.Length - body);
                var clip = new Clip { Format = f, Length = size, Data = Marshal.AllocHGlobal(size) };
                var dst = (short*)clip.Data;
                for (var i = 0; i < size / 2; i++)
                {
                    dst[i] = (short)Math.Clamp((int)(BitConverter.ToInt16(bytes, body + i * 2) * Volume), short.MinValue, short.MaxValue);
                }
                return clip;
            }
            o = body + size + (size & 1);
        }
        return null;
    }

    static void Scan()
    {
        s_scanned = true;
        try
        {
            if (!Directory.Exists(Folder))
            {
                Log($"no sounds folder ({Folder})");
                return;
            }
            foreach (var file in Directory.GetFiles(Folder, "*.wav"))
            {
                var name = Path.GetFileNameWithoutExtension(file).ToLowerInvariant().TrimEnd('0', '1', '2', '3', '4', '5', '6', '7', '8', '9', '_');
                try
                {
                    var clip = Load(file);
                    if (clip == null)
                    {
                        Log($"sound {Path.GetFileName(file)} isn't a 16-bit WAV");
                        continue;
                    }
                    if (!s_clips.TryGetValue(name, out var list))
                    {
                        s_clips[name] = list = new List<Clip>();
                    }
                    list.Add(clip);
                }
                catch (Exception e)
                {
                    Log($"sound {Path.GetFileName(file)}: {e.Message}");
                }
            }
            Debug.Log($"SkateCity: sounds: {string.Join(", ", s_clips.Select(kv => $"{kv.Key} x{kv.Value.Count}"))}");
        }
        catch (Exception e)
        {
            Log($"sounds folder: {e.Message}");
        }
    }

    public static void Preload()
    {
        if (!s_scanned)
        {
            Scan();
        }
    }

    /// Frees the voices that have finished.
    static void Reap(bool all = false)
    {
        for (var i = s_voices.Count - 1; i >= 0; i--)
        {
            var v = s_voices[i];
            if (!all && (v.Header->Flags & WhdrDone) == 0)
            {
                continue;
            }
            if (all)
            {
                waveOutReset(v.Handle);
            }
            waveOutUnprepareHeader(v.Handle, v.Header, sizeof(WaveHdr));
            waveOutClose(v.Handle);
            Marshal.FreeHGlobal((IntPtr)v.Header);
            s_voices.RemoveAt(i);
        }
    }

    public static bool Has(string name)
    {
        if (!s_scanned)
        {
            Scan();
        }
        return s_clips.TryGetValue(name, out var list) && list.Count > 0;
    }

    public static void Play(string name)
    {
        try
        {
            if (!s_scanned)
            {
                Scan();
            }
            Reap();
            if (!s_clips.TryGetValue(name, out var list) || list.Count == 0)
            {
                return;
            }
            if (s_voices.Count >= MaxVoices)
            {
                return;
            }
            var clip = list[s_rng.Next(list.Count)];
            var format = clip.Format;
            var err = waveOutOpen(out var handle, WaveMapper, ref format, IntPtr.Zero, IntPtr.Zero, 0);
            if (err != 0)
            {
                Log($"couldn't open sound output ({err})");
                return;
            }
            var header = (WaveHdr*)Marshal.AllocHGlobal(sizeof(WaveHdr));
            *header = new WaveHdr { Data = clip.Data, Length = (uint)clip.Length };
            waveOutPrepareHeader(handle, header, sizeof(WaveHdr));
            err = waveOutWrite(handle, header, sizeof(WaveHdr));
            s_voices.Add(new Voice { Handle = handle, Header = header });
            if (err != 0)
            {
                Log($"couldn't play sound {name} ({err})");
            }
        }
        catch (Exception e)
        {
            Log($"sound {name}: {e.Message}");
        }
    }
}


/// The park builder: ramps, boxes, ledges and rails you place around
/// Arkham City, like Skate 3's park editor. Pieces are drawn with the game's
/// box mesh and are solid to the skate engine's ground scan (the rays the
/// helper asks for hit them), so they skate like real geometry. They're saved
/// to Binaries\Win32\SkateCity\park.txt.
///
/// On foot, LB + D-pad down opens and closes it. Then: D-pad left/right picks
/// the piece, D-pad up/down raises and lowers it, Y turns it, X places it,
/// B removes the nearest one. Batman walks around to move the spot.
static unsafe class Park
{
    public enum Kind { Kicker, BigKicker, Box, Ledge, Rail, LongRail, QuarterPipe, Bank }

    static readonly string[] Names = { "Kicker", "Big kicker", "Manual box", "Ledge", "Flat rail", "Long rail", "Quarter pipe", "Bank" };

    public sealed class Piece
    {
        public Kind Kind;
        public Vector3 At;     // base point on the ground (game units)
        public float Yaw;      // degrees; the rider rolls in along +X
    }

    /// One solid box: centre, axes and half sizes, in game units.
    readonly struct Solid
    {
        public readonly Vector3 Centre, X, Y, Z, Half;
        public Solid(Vector3 c, Vector3 x, Vector3 y, Vector3 z, Vector3 half)
        {
            Centre = c; X = x; Y = y; Z = z; Half = half;
        }
    }

    static readonly List<Piece> s_pieces = new();
    static readonly List<Solid> s_solids = new();
    static readonly List<DynamicSMActor_Spawnable> s_actors = new();
    static readonly List<DynamicSMActor_Spawnable> s_ghost = new();
    static StaticMesh? s_cube;
    static Vector3 s_cubeExtent;
    static bool s_loaded, s_meshSearched, s_dirty = true;
    static float s_unitsPerMetre = 0f;

    // Editor state.
    public static bool Editing;
    static Kind s_kind = Kind.Kicker;
    static float s_raise, s_turn;
    static Piece? s_cursor;
    static ushort s_prevButtons;
    static long s_noticeAt;
    static string s_notice = "";

    static string FilePath => Path.Combine(Path.GetDirectoryName(Environment.ProcessPath) ?? ".", "SkateCity", "park.txt");

    public static float U => s_unitsPerMetre > 1f ? s_unitsPerMetre : 100f;

    /// Scan rays that hit the park since the last batch (for the log).
    public static int Hits;

    // ---- Shapes (metres, in the piece's own frame: X forward, Z up) ----

    /// A box from its centre (local metres), size (full, metres) and pitch
    /// (degrees, nose up along +X).
    static void AddBox(List<(Vector3 C, Vector3 Size, float Pitch)> into, Vector3 centre, Vector3 size, float pitch = 0f)
        => into.Add((centre, size, pitch));

    static List<(Vector3 C, Vector3 Size, float Pitch)> Shape(Kind kind)
    {
        var b = new List<(Vector3, Vector3, float)>();
        const float slab = 0.12f;
        void Ramp(float length, float angle, float width)
        {
            var a = angle * MathF.PI / 180f;
            // The riding surface: its top face starts at the ground.
            var top = new Vector3(MathF.Cos(a), 0, MathF.Sin(a)) * (length / 2f);
            var down = new Vector3(-MathF.Sin(a), 0, MathF.Cos(a)) * (slab / 2f);
            AddBox(b, top - down, new Vector3(length, width, slab), angle);
            // The back wall under its high end.
            var h = length * MathF.Sin(a);
            var x = length * MathF.Cos(a);
            AddBox(b, new Vector3(x - 0.05f, 0, h / 2f), new Vector3(0.1f, width, h));
        }
        switch (kind)
        {
            case Kind.Kicker:
                Ramp(2.0f, 18f, 1.8f);
                break;
            case Kind.BigKicker:
                Ramp(3.0f, 28f, 2.2f);
                break;
            case Kind.Bank:
                Ramp(4.0f, 22f, 3.0f);
                AddBox(b, new Vector3(4.0f * MathF.Cos(0.384f) + 1.0f, 0, 4.0f * MathF.Sin(0.384f) / 2f), new Vector3(2.0f, 3.0f, 4.0f * MathF.Sin(0.384f)));
                break;
            case Kind.Box:
                AddBox(b, new Vector3(1.5f, 0, 0.225f), new Vector3(3.0f, 1.2f, 0.45f));
                break;
            case Kind.Ledge:
                AddBox(b, new Vector3(2.0f, 0, 0.3f), new Vector3(4.0f, 0.5f, 0.6f));
                break;
            case Kind.Rail:
            case Kind.LongRail:
            {
                var length = kind == Kind.Rail ? 4f : 8f;
                AddBox(b, new Vector3(length / 2f, 0, 0.45f), new Vector3(length, 0.08f, 0.08f));
                for (var x = 0.4f; x < length; x += 3.2f)
                {
                    AddBox(b, new Vector3(x, 0, 0.205f), new Vector3(0.06f, 0.06f, 0.41f));
                }
                AddBox(b, new Vector3(length - 0.4f, 0, 0.205f), new Vector3(0.06f, 0.06f, 0.41f));
                break;
            }
            case Kind.QuarterPipe:
            {
                // Under 2 m tall: the scan counts anything higher as overhead.
                const float r = 1.7f, width = 3.0f;
                const int steps = 6;
                var last = 84f * MathF.PI / 180f;
                Vector3 P(float t) => new(r * MathF.Sin(t), 0, r * (1 - MathF.Cos(t)));
                for (var i = 0; i < steps; i++)
                {
                    var t0 = last * i / steps;
                    var t1 = last * (i + 1) / steps;
                    var a = P(t0);
                    var c = P(t1);
                    var mid = (a + c) / 2f;
                    var len = (c - a).Length() + 0.04f;
                    var pitch = MathF.Atan2(c.Z - a.Z, c.X - a.X);
                    var down = new Vector3(-MathF.Sin(pitch), 0, MathF.Cos(pitch)) * (slab / 2f);
                    AddBox(b, mid - down, new Vector3(len, width, slab), pitch * 180f / MathF.PI);
                }
                var topZ = r * (1 - MathF.Cos(last));
                var edge = r * MathF.Sin(last);
                // The deck behind the coping, and the wall under it.
                AddBox(b, new Vector3(edge + 0.75f, 0, topZ - 0.06f), new Vector3(1.5f, width, 0.12f));
                AddBox(b, new Vector3(edge + 1.45f, 0, topZ / 2f), new Vector3(0.1f, width, topZ));
                break;
            }
        }
        return b;
    }

    static IEnumerable<Solid> Solids(Piece piece)
    {
        var u = U;
        var yaw = piece.Yaw * MathF.PI / 180f;
        var fx = new Vector3(MathF.Cos(yaw), MathF.Sin(yaw), 0);
        var fy = new Vector3(-MathF.Sin(yaw), MathF.Cos(yaw), 0);
        foreach (var (c, size, pitchDeg) in Shape(piece.Kind))
        {
            var p = pitchDeg * MathF.PI / 180f;
            var x = fx * MathF.Cos(p) + Vector3.UnitZ * MathF.Sin(p);
            var z = -fx * MathF.Sin(p) + Vector3.UnitZ * MathF.Cos(p);
            var centre = piece.At + (fx * c.X + fy * c.Y + Vector3.UnitZ * c.Z) * u;
            yield return new Solid(centre, x, fy, z, size * (u / 2f));
        }
    }

    // ---- The ground scan sees the park ----

    /// The nearest point along from..to where the ray enters a park piece.
    public static bool Raycast(Vector3 from, Vector3 to, out Vector3 at)
    {
        at = default;
        if (s_solids.Count == 0)
        {
            return false;
        }
        var d = to - from;
        var best = 2f;
        foreach (var s in s_solids)
        {
            var o = from - s.Centre;
            float t0 = 0f, t1 = 1f;
            var hit = true;
            for (var k = 0; k < 3 && hit; k++)
            {
                var axis = k == 0 ? s.X : k == 1 ? s.Y : s.Z;
                var half = k == 0 ? s.Half.X : k == 1 ? s.Half.Y : s.Half.Z;
                var os = Vector3.Dot(o, axis);
                var ds = Vector3.Dot(d, axis);
                if (MathF.Abs(ds) < 1e-6f)
                {
                    hit = MathF.Abs(os) <= half;
                    continue;
                }
                var a = (-half - os) / ds;
                var b = (half - os) / ds;
                if (a > b)
                {
                    (a, b) = (b, a);
                }
                t0 = MathF.Max(t0, a);
                t1 = MathF.Min(t1, b);
                hit = t0 <= t1;
            }
            if (hit && t0 > 0f && t0 < best)
            {
                best = t0;
            }
        }
        if (best > 1f)
        {
            return false;
        }
        at = from + d * best;
        return true;
    }

    // ---- Saving ----

    static void Load()
    {
        s_loaded = true;
        try
        {
            if (!File.Exists(FilePath))
            {
                return;
            }
            foreach (var line in File.ReadAllLines(FilePath))
            {
                var f = line.Split(' ', StringSplitOptions.RemoveEmptyEntries);
                if (f.Length < 5 || line.StartsWith('#') || !System.Enum.TryParse<Kind>(f[0], out var kind))
                {
                    continue;
                }
                var inv = System.Globalization.CultureInfo.InvariantCulture;
                s_pieces.Add(new Piece
                {
                    Kind = kind,
                    At = new Vector3(float.Parse(f[1], inv), float.Parse(f[2], inv), float.Parse(f[3], inv)),
                    Yaw = float.Parse(f[4], inv),
                });
            }
            Debug.Log($"SkateCity: park: {s_pieces.Count} pieces loaded");
        }
        catch (Exception e)
        {
            Debug.Log($"SkateCity: park file: {e.Message}");
        }
    }

    static void Save()
    {
        try
        {
            var inv = System.Globalization.CultureInfo.InvariantCulture;
            var lines = new List<string> { "# SkateCity park: kind x y z yaw (game units, degrees)" };
            foreach (var p in s_pieces)
            {
                lines.Add(string.Format(inv, "{0} {1:F1} {2:F1} {3:F1} {4:F1}", p.Kind, p.At.X, p.At.Y, p.At.Z, p.Yaw));
            }
            Directory.CreateDirectory(Path.GetDirectoryName(FilePath)!);
            File.WriteAllLines(FilePath, lines);
        }
        catch (Exception e)
        {
            Debug.Log($"SkateCity: saving the park: {e.Message}");
        }
    }

    // ---- Drawing the pieces ----

    static bool FindMesh()
    {
        if (!s_meshSearched)
        {
            s_meshSearched = true;
            foreach (var path in new[] { "EngineMeshes.Cube", "EditorMeshes.TexPropCube", "EngineMeshes.ParticleCube", "EditorMeshes.EditorCube" })
            {
                try
                {
                    s_cube = Game.FindObject<StaticMesh>(path) ?? GameObject.DynamicLoadObject(path, StaticMesh.StaticClass(), true) as StaticMesh;
                }
                catch
                {
                }
                if (s_cube != null)
                {
                    break;
                }
            }
            if (s_cube == null)
            {
                Debug.Log("SkateCity: park: no box mesh found; pieces will be invisible");
            }
        }
        return s_cube != null;
    }

    static void Draw(List<DynamicSMActor_Spawnable> pool, IList<Solid> solids)
    {
        if (!FindMesh())
        {
            return;
        }
        for (var i = 0; i < solids.Count; i++)
        {
            var s = solids[i];
            if (i >= pool.Count || !pool[i].IsValid)
            {
                var actor = new DynamicSMActor_Spawnable(s.Centre, default);
                actor.SetCollision(false, false, false);
                actor.StaticMeshComponent.SetStaticMesh(s_cube!, false);
                if (i < pool.Count)
                {
                    pool[i] = actor;
                }
                else
                {
                    pool.Add(actor);
                }
            }
            var part = pool[i];
            if (s_cubeExtent.X < 0.01f)
            {
                part.SetRotation(default);
                part.StaticMeshComponent.SetScale3D(Vector3.One);
                var e = part.StaticMeshComponent.Bounds.BoxExtent;
                if (e.X > 0.01f && e.Y > 0.01f && e.Z > 0.01f)
                {
                    s_cubeExtent = e;
                }
            }
            if (s_cubeExtent.X >= 0.01f)
            {
                part.StaticMeshComponent.SetScale3D(new Vector3(s.Half.X / s_cubeExtent.X, s.Half.Y / s_cubeExtent.Y, s.Half.Z / s_cubeExtent.Z));
            }
            part.SetLocation(s.Centre);
            var pitch = MathF.Asin(Math.Clamp(s.X.Z, -1f, 1f)) * 180f / MathF.PI;
            var yaw = MathF.Atan2(s.Y.Y, s.Y.X) * 180f / MathF.PI - 90f;
            part.SetRotation(new Rotator(pitch, yaw, 0f));
            part.SetHidden(false);
        }
        for (var i = solids.Count; i < pool.Count; i++)
        {
            if (pool[i].IsValid)
            {
                pool[i].SetHidden(true);
            }
        }
    }

    static void Rebuild()
    {
        s_dirty = false;
        s_solids.Clear();
        foreach (var p in s_pieces)
        {
            s_solids.AddRange(Solids(p));
        }
        Draw(s_actors, s_solids);
    }

    /// A new world: the old actors went with it.
    public static void NewWorld()
    {
        s_actors.Clear();
        s_ghost.Clear();
        s_dirty = true;
        Editing = false;
    }

    // ---- Each frame ----

    [DllImport("xinput1_4.dll", EntryPoint = "XInputGetState")]
    static extern int XInputGetState(int index, out XState state);

    [StructLayout(LayoutKind.Sequential)]
    struct XState
    {
        public uint Packet;
        public ushort Buttons;
        public byte LT, RT;
        public short LX, LY, RX, RY;
    }

    [DllImport("user32.dll")]
    static extern IntPtr GetForegroundWindow();

    [DllImport("user32.dll")]
    static extern uint GetWindowThreadProcessId(IntPtr window, out uint processId);

    public static bool GameFocused()
    {
        GetWindowThreadProcessId(GetForegroundWindow(), out var pid);
        return pid == (uint)Environment.ProcessId;
    }

    const ushort DUp = 0x0001, DDown = 0x0002, DLeft = 0x0004, DRight = 0x0008, LB = 0x0100, A = 0x1000, B = 0x2000, X = 0x4000, Y = 0x8000;

    static void Notice(string text)
    {
        s_notice = text;
        s_noticeAt = Environment.TickCount64;
    }

    /// Player 1's link calls this every frame. `skating`: on the board.
    public static void Tick(RPawnPlayer? pawn, bool skating, bool focused)
    {
        if (!s_loaded)
        {
            Load();
        }
        if (pawn != null && pawn.IsValid && s_unitsPerMetre < 1f)
        {
            try
            {
                var cyl = pawn.CylinderComponent;
                if (cyl != null && cyl.IsValid && cyl.CollisionHeight > 1f)
                {
                    s_unitsPerMetre = Math.Clamp(cyl.CollisionHeight * 2f / 1.85f, 30f, 300f);
                    s_dirty = true;
                }
            }
            catch
            {
            }
        }
        if (s_dirty)
        {
            Rebuild();
        }

        ushort buttons = 0;
        if (focused && XInputGetState(0, out var state) == 0)
        {
            buttons = state.Buttons;
        }
        var pressed = (ushort)(buttons & ~s_prevButtons);
        s_prevButtons = buttons;

        if (skating)
        {
            if (Editing)
            {
                Close();
            }
            return;
        }
        if ((buttons & LB) != 0 && (pressed & DDown) != 0)
        {
            if (Editing)
            {
                Close();
            }
            else
            {
                Editing = true;
                Notice("Park builder");
                Debug.Log("SkateCity: park builder open");
            }
            return;
        }
        if (!Editing || pawn == null || !pawn.IsValid)
        {
            return;
        }
        var lb = (buttons & LB) != 0;
        var count = System.Enum.GetValues<Kind>().Length;
        if (!lb && (pressed & DRight) != 0)
        {
            s_kind = (Kind)(((int)s_kind + 1) % count);
        }
        if (!lb && (pressed & DLeft) != 0)
        {
            s_kind = (Kind)(((int)s_kind + count - 1) % count);
        }
        if (!lb && (pressed & DUp) != 0)
        {
            s_raise = MathF.Min(s_raise + 0.25f, 6f);
        }
        if (!lb && (pressed & DDown) != 0)
        {
            s_raise = MathF.Max(s_raise - 0.25f, 0f);
        }
        if ((pressed & Y) != 0)
        {
            s_turn = (s_turn + 45f) % 360f;
        }

        // The spot: 3 m in front of Batman, on whatever is under it.
        var u = U;
        var yaw = pawn.Rotation.Yaw;
        var forward = new Rotator(0f, yaw, 0f).ToDirection();
        var spot = pawn.Location + forward * (3f * u);
        var feet = pawn.Location.Z - (pawn.CylinderComponent?.CollisionHeight ?? u);
        var ground = feet;
        try
        {
            var top = new Vector3(spot.X, spot.Y, feet + 2.5f * u);
            var hitActor = pawn.Trace(out var hit, out _, top - Vector3.UnitZ * (8f * u), top, false, Vector3.Zero, out _, 0);
            if (hitActor != null)
            {
                ground = hit.Z;
            }
            if (Raycast(top, top - Vector3.UnitZ * (8f * u), out var onPark) && onPark.Z > ground)
            {
                ground = onPark.Z;
            }
        }
        catch
        {
        }
        s_cursor = new Piece { Kind = s_kind, At = new Vector3(spot.X, spot.Y, ground + s_raise * u), Yaw = yaw + s_turn };
        Draw(s_ghost, Solids(s_cursor).ToList());

        if ((pressed & X) != 0)
        {
            s_pieces.Add(new Piece { Kind = s_cursor.Kind, At = s_cursor.At, Yaw = s_cursor.Yaw });
            Save();
            Rebuild();
            Notice($"Placed: {Names[(int)s_kind]}");
        }
        if ((pressed & B) != 0 && s_pieces.Count > 0)
        {
            var nearest = s_pieces.MinBy(p => Vector3.DistanceSquared(p.At, s_cursor.At))!;
            if (Vector3.Distance(nearest.At, s_cursor.At) < 6f * u)
            {
                s_pieces.Remove(nearest);
                Save();
                Rebuild();
                Notice($"Removed: {Names[(int)nearest.Kind]}");
            }
        }
    }

    static void Close()
    {
        Editing = false;
        s_cursor = null;
        Draw(s_ghost, new List<Solid>());
        Save();
        Notice($"Park saved ({s_pieces.Count} pieces)");
        Debug.Log($"SkateCity: park builder closed, {s_pieces.Count} pieces");
    }

    /// The builder's help text and notices.
    public static void DrawHud(Canvas canvas, float w, float h, float ui, Font? font)
    {
        var now = Environment.TickCount64;
        var lines = new List<string>();
        if (Editing)
        {
            lines.Add($"PARK BUILDER   -   {Names[(int)s_kind]}   -   height {s_raise:F2} m   -   {s_pieces.Count} placed");
            lines.Add("D-pad left/right: piece   D-pad up/down: height   Y: turn   X: place   B: remove nearest");
            lines.Add("Walk to move the spot   -   LB + D-pad down: done");
        }
        if (s_notice.Length > 0 && now - s_noticeAt < 2500)
        {
            lines.Add(s_notice);
        }
        if (lines.Count == 0)
        {
            return;
        }
        if (font != null)
        {
            canvas.Font = font;
        }
        var y = h * 0.08f;
        foreach (var line in lines)
        {
            canvas.SetPos(w * 0.03f + 2, y + 2, 0);
            canvas.SetDrawColor(0, 0, 0, 200);
            canvas.DrawText(line, false, 0.7f * ui, 0.7f * ui, out _);
            canvas.SetPos(w * 0.03f, y, 0);
            canvas.SetDrawColor(255, 214, 80, 240);
            canvas.DrawText(line, false, 0.7f * ui, 0.7f * ui, out _);
            y += 30f * ui;
        }
    }
}
