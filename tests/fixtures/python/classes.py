class Animal:
    """A base animal class."""

    def __init__(self, name):
        self.name = name

    def speak(self):
        pass

    async def move(self):
        pass


class Dog(Animal):
    def speak(self):
        return "Woof!"
