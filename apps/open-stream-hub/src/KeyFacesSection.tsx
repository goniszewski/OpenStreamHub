import { useEffect, useMemo, useRef } from 'react';
import { ArrowDown, ArrowUp, Plus, Trash2 } from 'lucide-react';
import type { KeyFace, KeypadSlot, Profile } from './contracts';
import { parseHexColour } from './artwork-model';
import { ColourPicker } from './ColourPicker';
import { SelectField } from './controls';
import { keyFaceIssue, maximumKeyFaces } from './draft-model';
import { useT } from './i18n-react';
import { drawActionKey, keyFaceRgba, keyFaceSide, tileIconOf } from './key-face';
import type { TileIcon } from './key-face';
import { canAddFace, faceComparisons, faceToRow, moveFace, newFaceRow, rowToFace, setKeyFaces, withVariable } from './key-faces-model';
import type { FaceComparison, FaceRow } from './key-faces-model';
import type { ProfileSession } from './profile-session';
import { VariableSelect, VariableValueField } from './VariableFields';
import { useVariables } from './variables-context';

type ActionSlot = Extract<KeypadSlot, { type: 'action' }>;

// The face as the module draws it: the tile's icon under the face's label (else the tile's) on its background.
function FacePreview({ icon, label, background, name }: { icon: TileIcon; label: string; background: string; name: string }) {
  const face = useMemo(() => drawActionKey(icon, label, parseHexColour(background) ?? undefined), [icon, label, background]);
  const canvas = useRef<HTMLCanvasElement>(null);
  useEffect(() => {
    // jsdom has no 2D context; the preview is then left blank.
    const context = typeof ImageData === 'undefined' ? null : canvas.current?.getContext('2d');
    if (context) context.putImageData(new ImageData(keyFaceRgba(face), keyFaceSide, keyFaceSide), 0, 0);
  }, [face]);
  return <canvas ref={canvas} className="key-face key-face-preview" width={keyFaceSide} height={keyFaceSide} role="img" aria-label={name} />;
}

// What an action tile shows while a variable test holds: up to eight faces, the first that holds wins. Every change is
// one edit of the draft (and one undo step); typing in the same field of the same face coalesces.
export function KeyFacesSection({ profile, session, path, control, slot, disabled }: {
  profile: Profile; session: Pick<ProfileSession, 'edit'>; path: readonly string[]; control: number; slot: ActionSlot; disabled: boolean;
}) {
  const t = useT();
  const variables = useVariables();
  const faces = slot.faces ?? [];
  const place = `${path.join('/')}:${control}`;
  const icon = tileIconOf(slot.bindings[0]?.action);
  const commit = (next: KeyFace[], field?: string) => session.edit(setKeyFaces(profile, path, control, slot, next), field === undefined ? undefined : `key-faces:${place}:${field}`);
  const replace = (index: number, row: FaceRow, field: string) => commit(faces.map((face, at) => (at === index ? rowToFace(row) : face)), `${index}:${field}`);
  return <section className="key-faces" aria-labelledby="key-faces-heading">
    <h3 id="key-faces-heading">{t('faces.heading')}</h3>
    <p className="fine-print">{t('faces.intro')}</p>
    {faces.length > 0 && <ol className="key-faces-list">
      {faces.map((face, index) => {
        const id = (name: string) => `key-face-${index}-${name}`;
        const row = faceToRow(face);
        const issue = keyFaceIssue(face, variables);
        const number = index + 1;
        return <li key={index}><fieldset disabled={disabled}>
          <div className="key-face-row-head">
            <strong>{t('faces.face', { number })}</strong>
            <FacePreview icon={icon} label={face.label ?? slot.label} background={face.background ?? ''} name={t('faces.preview', { number })} />
          </div>
          {row ? <>
            <VariableSelect id={id('variable')} label={t('faces.variable')} value={row.variable} kinds={['bool', 'number', 'text']}
              onChange={name => replace(index, withVariable(row, name, variables), 'variable')} />
            <label className="spaced-label" htmlFor={id('comparison')}>{t('faces.comparison')}</label>
            <SelectField id={id('comparison')} value={row.comparison} disabled={disabled}
              onChange={event => replace(index, { ...row, comparison: event.target.value as FaceComparison }, 'comparison')}>
              {faceComparisons(row, variables).map(item => <option key={item} value={item}>{t(item === 'is' ? 'faces.is' : 'faces.atLeast')}</option>)}
            </SelectField>
            <VariableValueField id={id('value')} label={t('faces.value')} spaced value={row.value} onChange={value => replace(index, { ...row, value }, 'value')} />
            <label className="spaced-label" htmlFor={id('label')}>{t('faces.label')}</label>
            <input id={id('label')} disabled={disabled} value={row.label} placeholder={slot.label} onChange={event => replace(index, { ...row, label: event.target.value }, 'label')} />
            <label className="key-face-background-toggle spaced-label">
              <input type="checkbox" id={id('background-on')} checked={row.background !== ''} disabled={disabled}
                onChange={event => replace(index, { ...row, background: event.target.checked ? '#c81e1e' : '' }, 'background')} />
              {t('faces.background')}
            </label>
            {row.background !== '' && <ColourPicker id={id('background')} value={row.background} disabled={disabled} onChange={hex => replace(index, { ...row, background: hex }, 'background')} />}
          </> : <p className="fine-print" role="note">{t('faces.combined')}</p>}
          {issue && <p id={id('error')} className="field-error" role="alert">{issue}</p>}
          <div className="key-face-buttons">
            <button type="button" className="button compact" disabled={disabled || index === 0} aria-label={t('faces.moveUp', { number })} title={t('faces.moveUp', { number })}
              onClick={() => commit(moveFace(faces, index, -1))}><ArrowUp size={16} /></button>
            <button type="button" className="button compact" disabled={disabled || index === faces.length - 1} aria-label={t('faces.moveDown', { number })} title={t('faces.moveDown', { number })}
              onClick={() => commit(moveFace(faces, index, 1))}><ArrowDown size={16} /></button>
            <button type="button" className="button compact" disabled={disabled} aria-label={t('faces.removeNamed', { number })}
              onClick={() => commit(faces.filter((_, at) => at !== index))}><Trash2 size={16} />{t('faces.remove')}</button>
          </div>
        </fieldset></li>;
      })}
    </ol>}
    <button type="button" id="key-face-add" className="button compact" disabled={disabled || !canAddFace(faces)}
      onClick={() => commit([...faces, rowToFace(newFaceRow(variables))])}><Plus size={16} />{t('faces.add')}</button>
    {!canAddFace(faces) && <p className="fine-print">{t('faces.limit', { max: maximumKeyFaces })}</p>}
  </section>;
}
