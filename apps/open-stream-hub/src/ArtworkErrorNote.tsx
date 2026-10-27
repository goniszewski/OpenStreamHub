import type { ArtworkNotice } from './artwork-error-model';

/** The plain-language message as the alert, with the raw backend text one click away to read or copy. */
export function ArtworkErrorNote({ notice }: { notice: ArtworkNotice }) {
  return <>
    <p className="field-error" role="alert">{notice.message}</p>
    {notice.detail && <details className="field-detail"><summary>Details</summary><p>{notice.detail}</p></details>}
  </>;
}
