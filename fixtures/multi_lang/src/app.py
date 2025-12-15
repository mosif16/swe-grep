from typing import Protocol, runtime_checkable
import os
import sys as system


def deco(fn):
    return fn


@runtime_checkable
class Runner(Protocol):
    def run(self, x: int) -> int: ...


class Base:
    pass


@deco
class Job(Base):
    def __init__(self):
        pass

    def run(self, x: int) -> int:
        helper()
        return x


def helper():
    print("hi")


async def main():
    j = Job()
    j.run(1)
    os.getcwd()
    system.exit(0)
