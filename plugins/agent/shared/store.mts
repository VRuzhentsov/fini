// The SQLite store, through Node's built-in node:sqlite: no npm package and no native module.
import { DatabaseSync } from 'node:sqlite';
import { chmodSync, mkdirSync } from 'node:fs';
import { dirname } from 'node:path';

export type Session = { agent: string; session_id: string };
export type Relation = Session & { quest_id: string; is_primary: boolean | null };

export interface RelationsRepository {
  link(quest: string, session: Session): boolean;
  unlink(quest: string, session: Session): boolean;
  promote(quest: string, session: Session): void;
  sessions(quest: string): Relation[];
  quests(session: Session): Relation[];
  close(): void;
}

type Row = { quest_id: string; agent: string; session_id: string; is_primary: number | null };
const relation = (row: Row): Relation => ({ quest_id: row.quest_id, agent: row.agent,
  session_id: row.session_id, is_primary: row.is_primary === 1 ? true : null });

export class SqliteRelationsRepository implements RelationsRepository {
  private db: DatabaseSync;

  constructor(path: string) {
    mkdirSync(dirname(path), { recursive: true, mode: 0o700 });
    this.db = new DatabaseSync(path);
    try {
      chmodSync(path, 0o600);
      // SQLite owns uniqueness and serializes first-link and primary changes.
      this.db.exec(`
        PRAGMA busy_timeout = 8000;
        CREATE TABLE IF NOT EXISTS relations (
          quest_id TEXT NOT NULL, agent TEXT NOT NULL, session_id TEXT NOT NULL,
          is_primary INTEGER CHECK (is_primary IS NULL OR is_primary = 1),
          PRIMARY KEY (quest_id, agent, session_id)
        );
        CREATE UNIQUE INDEX IF NOT EXISTS relations_primary
          ON relations (quest_id, agent) WHERE is_primary = 1;
        CREATE INDEX IF NOT EXISTS relations_session ON relations (agent, session_id);
      `);
    } catch (error) { this.db.close(); throw error; }
  }

  // An immediate transaction takes the write lock first, so two links cannot both be first.
  private transaction<T>(work: () => T): T {
    this.db.exec('BEGIN IMMEDIATE');
    try {
      const result = work();
      this.db.exec('COMMIT');
      return result;
    } catch (error) { this.db.exec('ROLLBACK'); throw error; }
  }

  link(quest: string, session: Session): boolean {
    return this.transaction(() => {
      const first = this.db.prepare('SELECT 1 FROM relations WHERE quest_id = ? AND agent = ? LIMIT 1')
        .get(quest, session.agent) === undefined;
      return Number(this.db.prepare(`INSERT INTO relations (quest_id, agent, session_id, is_primary)
        VALUES (?, ?, ?, ?) ON CONFLICT DO NOTHING`)
        .run(quest, session.agent, session.session_id, first ? 1 : null).changes) === 1;
    });
  }

  unlink(quest: string, session: Session): boolean {
    return Number(this.db.prepare('DELETE FROM relations WHERE quest_id = ? AND agent = ? AND session_id = ?')
      .run(quest, session.agent, session.session_id).changes) === 1;
  }

  promote(quest: string, session: Session): void {
    this.transaction(() => {
      if (this.db.prepare('SELECT 1 FROM relations WHERE quest_id = ? AND agent = ? AND session_id = ?')
        .get(quest, session.agent, session.session_id) === undefined) {
        throw new Error('Link this session before making it primary.');
      }
      this.db.prepare('UPDATE relations SET is_primary = NULL WHERE quest_id = ? AND agent = ?').run(quest, session.agent);
      this.db.prepare('UPDATE relations SET is_primary = 1 WHERE quest_id = ? AND agent = ? AND session_id = ?')
        .run(quest, session.agent, session.session_id);
    });
  }

  sessions(quest: string): Relation[] {
    return (this.db.prepare(`SELECT * FROM relations WHERE quest_id = ?
      ORDER BY agent ASC, is_primary DESC, session_id ASC`).all(quest) as Row[]).map(relation);
  }

  quests(session: Session): Relation[] {
    return (this.db.prepare(`SELECT * FROM relations WHERE agent = ? AND session_id = ?
      ORDER BY quest_id ASC`).all(session.agent, session.session_id) as Row[]).map(relation);
  }

  close(): void { this.db.close(); }
}
