import React from "react";

export async function getUser(id: number): Promise<string> {
  return Promise.resolve(`user_${id}`);
}

export function filterActiveUsers(names: string[]): string[] {
  return names.filter((name) => name.startsWith("active"));
}

export interface Resource<T> {
  data: T;
  meta?: Record<string, unknown>;
}

export const UserCard: React.FC<{ user: Resource<string> }> = ({ user }) => (
  <section className="user-card">{user.data}</section>
);

export function withFallback<T>(value: T | undefined, fallback: T): T {
  return value ?? fallback;
}

type AsyncMapper<T, R> = (value: T) => Promise<R>;

export async function mapResources<T, R>(
  items: T[],
  mapper: AsyncMapper<T, R>
): Promise<R[]> {
  const results: R[] = [];
  for (const item of items) {
    results.push(await mapper(item));
  }
  return results;
}
