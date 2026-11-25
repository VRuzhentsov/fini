import Database from 'better-sqlite3';
import { and, asc, eq, sql } from 'drizzle-orm';
import { drizzle } from 'drizzle-orm/better-sqlite3';
import { integer, sqliteTable, text } from 'drizzle-orm/sqlite-core';
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

const relations = sqliteTable('relations', {
  quest_id: text().notNull(),
  agent: text().notNull(),
  session_id: text().notNull(),
  is_primary: integer({ mode: 'boolean' }),
});

export class SqliteRelationsRepository implements RelationsRepository {
  private connection: Database.Database;
  private db: ReturnType<typeof drizzle>;

  constructor(path: string) {
    mkdirSync(dirname(path), { recursive: true, mode: 0o700 });
    this.connection = new Database(path, { timeout: 8000 });
    try {
      chmodSync(path, 0o600);
      this.db = drizzle(this.connection);
      // SQLite owns uniqueness and serializes first-link and primary changes.
      this.connection.exec(`
        CREATE TABLE IF NOT EXISTS relations (
          quest_id TEXT NOT NULL, agent TEXT NOT NULL, session_id TEXT NOT NULL,
          is_primary INTEGER CHECK (is_primary IS NULL OR is_primary = 1),
          PRIMARY KEY (quest_id, agent, session_id)
        );
        CREATE UNIQUE INDEX IF NOT EXISTS relations_primary
          ON relations (quest_id, agent) WHERE is_primary = 1;
        CREATE INDEX IF NOT EXISTS relations_session ON relations (agent, session_id);
      `);
    } catch (error) { this.connection.close(); throw error; }
  }

  link(quest: string, session: Session): boolean {
    return this.db.transaction(tx => {
      const group = and(eq(relations.quest_id, quest), eq(relations.agent, session.agent));
      const first = tx.select().from(relations).where(group).limit(1).get() === undefined;
      return tx.insert(relations).values({ quest_id: quest, ...session, is_primary: first ? true : null })
        .onConflictDoNothing().run().changes === 1;
    }, { behavior: 'immediate' });
  }

  unlink(quest: string, session: Session): boolean {
    return this.db.delete(relations).where(and(eq(relations.quest_id, quest),
      eq(relations.agent, session.agent), eq(relations.session_id, session.session_id))).run().changes === 1;
  }

  promote(quest: string, session: Session): void {
    this.db.transaction(tx => {
      const group = and(eq(relations.quest_id, quest), eq(relations.agent, session.agent));
      const target = and(group, eq(relations.session_id, session.session_id));
      if (!tx.select().from(relations).where(target).get()) throw new Error('Link this session before making it primary.');
      tx.update(relations).set({ is_primary: null }).where(group).run();
      tx.update(relations).set({ is_primary: true }).where(target).run();
    }, { behavior: 'immediate' });
  }

  sessions(quest: string): Relation[] {
    return this.db.select().from(relations).where(eq(relations.quest_id, quest))
      .orderBy(asc(relations.agent), sql`${relations.is_primary} DESC`, asc(relations.session_id)).all();
  }

  quests(session: Session): Relation[] {
    return this.db.select().from(relations)
      .where(and(eq(relations.agent, session.agent), eq(relations.session_id, session.session_id)))
      .orderBy(asc(relations.quest_id)).all();
  }

  close(): void { this.connection.close(); }
}
