export const IPC_CHANNELS = {
  audio: {
    loadTrack: 'audio:load-track',
    play: 'audio:play',
    stop: 'audio:stop',
    getState: 'audio:get-state',
    seek: 'audio:seek',
    setCrossfader: 'audio:set-crossfader',
    setMasterTempo: 'audio:set-master-tempo',
    setDeckCue: 'audio:set-deck-cue',
    setEqCut: 'audio:set-eq-cut',
    setDeckGain: 'audio:set-deck-gain',
    setBeatLoop: 'audio:set-beat-loop',
    clearLoop: 'audio:clear-loop',
    startDeck: 'audio:start-deck',
    setMicEnabled: 'audio:set-mic-enabled',
    getDevices: 'audio:get-devices',
    getConfig: 'audio:get-config',
    updateConfig: 'audio:update-config',
  },
  recording: {
    getConfig: 'recording:get-config',
    updateConfig: 'recording:update-config',
    getStatus: 'recording:get-status',
    start: 'recording:start',
    stop: 'recording:stop',
  },
  nativeUi: {
    attach: 'native-ui:attach',
    setFrame: 'native-ui:set-frame',
    setWaveform: 'native-ui:set-waveform',
    detach: 'native-ui:detach',
    setArtwork: 'native-ui:set-artwork',
    clearArtwork: 'native-ui:clear-artwork',
  },
  system: {
    getInfo: 'system:get-info',
  },
  osc: {
    getConfig: 'osc:get-config',
    updateConfig: 'osc:update-config',
  },
} as const;

export const IPC_EVENTS = {
  notification: 'notification',
  trackEnded: 'track-ended',
  waveformChunk: 'waveform-chunk',
  waveformComplete: 'waveform-complete',
  trackStructure: 'track-structure',
  recordingStatus: 'recording-status',
} as const;
