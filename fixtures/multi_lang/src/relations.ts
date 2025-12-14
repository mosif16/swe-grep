export interface Animal {
  name: string;
}

export interface Dog extends Animal {
  bark(): void;
}

export class Cat implements Animal {
  name: string;

  constructor(name: string) {
    this.name = name;
  }
}

export class Poodle implements Dog {
  name: string;

  constructor(name: string) {
    this.name = name;
  }

  bark(): void {}
}
