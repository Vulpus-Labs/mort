-- Builds the project scripts/smoke.sh checks, then saves it as
-- $TMPDIR/mort/smoke.rpp so REAPER can quit without asking.
--
-- REAPER runs a .lua file given on its command line:
--   open -a REAPER --args -new "$PWD/scripts/smoke-project.lua"

local proj = 0
reaper.Undo_BeginBlock()
reaper.SetCurrentBPM(proj, 120, false)

local function add(name, idx)
  reaper.InsertTrackAtIndex(idx, true)
  local t = reaper.GetTrack(proj, idx)
  reaper.GetSetMediaTrackInfo_String(t, "P_NAME", name, true)
  return t
end

local drums = add("Drums", 0)
local kick = add("Kick", 1)
local bass = add("Bass", 2)
local verb = add("Verb", 3)

-- Kick sits in the Drums folder.
reaper.SetMediaTrackInfo_Value(drums, "I_FOLDERDEPTH", 1)
reaper.SetMediaTrackInfo_Value(kick, "I_FOLDERDEPTH", -1)

reaper.SetMediaTrackInfo_Value(bass, "D_VOL", 0.5)
reaper.SetMediaTrackInfo_Value(bass, "D_PAN", -0.2)
reaper.SetMediaTrackInfo_Value(bass, "B_MUTE", 1)
reaper.CreateTrackSend(kick, verb)

reaper.TrackFX_AddByName(bass, "ReaEQ", false, -1)
local comp = reaper.TrackFX_AddByName(bass, "ReaComp", false, -1)
reaper.TrackFX_SetEnabled(bass, comp, false)

-- 120 bpm to bar 9 (16 s), then 90 bpm.
reaper.AddTempoTimeSigMarker(proj, 16, 90, 4, 4, false)

reaper.AddProjectMarker2(proj, true, 0, 16, "verse", 1, 0)
reaper.AddProjectMarker2(proj, false, 8, 0, "drop", 2, 0)
reaper.CreateNewMIDIItemInProj(bass, 0, 4, false)

reaper.Undo_EndBlock("smoke project", -1)

local dir = (os.getenv("TMPDIR") or "/tmp/"):gsub("/?$", "/") .. "mort/"
reaper.RecursiveCreateDirectory(dir, 0)
reaper.Main_SaveProjectEx(proj, dir .. "smoke.rpp", 0)
