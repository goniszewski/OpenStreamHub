import { describe, expect, it } from 'vitest';
import { mediaKeyLabels, mediaLabels, obsMediaOpLabels, obsMuteOpLabels, obsOperationLabels, obsOutputOpLabels, obsRecordOpLabels, obsReplayBufferOpLabels, obsSwitchOpLabels, profileDirectionLabels, streamlabsAudioModeLabels, streamlabsOperationLabels, streamlabsVisibilityOpLabels, systemLabels, timerSoundLabels } from './action-model';
import { pickerEntries } from './action-picker-model';
import { variableKindLabels } from './flow-model';
import { chordLabel } from './key-capture-model';
import { keyTextAlignLabels } from './key-artwork-design';
import { knobOrientationLabels } from './knob-orientation';
import { lightingScopeLabels } from './lighting-model';
import { outcomeLabels } from './MhzImport';
import { obsConnectionLabels } from './obs-settings';
import { permissionLabels } from './PermissionsPanel';
import { reasonLabels } from './service-runner';
import { streamlabsConnectionLabels } from './streamlabs-settings';
import { gestureLabels, replayLabels } from './ui-model';

// `translate` falls back to the key itself, so a mistyped key would show up as 'labels.media.playPause'.
const catalogueKey = /^[a-z]+\.[A-Za-z.]+$/;

const maps: Record<string, Record<string, string>> = {
  mediaKeyLabels, mediaLabels, systemLabels, profileDirectionLabels, timerSoundLabels, obsOutputOpLabels, obsSwitchOpLabels,
  obsMuteOpLabels, obsMediaOpLabels, obsRecordOpLabels, obsReplayBufferOpLabels, obsOperationLabels, streamlabsOperationLabels,
  streamlabsVisibilityOpLabels, streamlabsAudioModeLabels, keyTextAlignLabels, variableKindLabels, knobOrientationLabels,
  lightingScopeLabels, obsConnectionLabels, streamlabsConnectionLabels, reasonLabels, gestureLabels, replayLabels, outcomeLabels,
  permissionLabels,
};

describe('label maps read from the catalogue', () => {
  it.each(Object.entries(maps))('%s has text for every key', (_, labels) => {
    for (const label of Object.values(labels)) {
      expect(label).not.toBe('');
      expect(label).not.toMatch(catalogueKey);
    }
  });

  it('gives every picker entry and modifier text', () => {
    for (const entry of pickerEntries) expect(entry.label).not.toMatch(catalogueKey);
    expect(chordLabel({ key: 'p', modifiers: ['command', 'control', 'option', 'shift'] })).toBe('Command+Control+Option+Shift+P');
  });
});
