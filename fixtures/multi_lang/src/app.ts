export async function getUser(id: number): Promise<string> {
  return Promise.resolve(`user_${id}`);
}

export function filterActiveUsers(names: string[]): string[] {
  return names.filter((name) => name.startsWith("active"));
}
