#include <string>

class Animal {
public:
    Animal(std::string name) : name_(name) {}
    virtual ~Animal() = default;

    virtual std::string speak() { return "..."; }
    std::string get_name() { return name_; }

private:
    std::string name_;
};

class Dog : public Animal {
public:
    Dog(std::string name) : Animal(name) {}

    std::string speak() override { return "Woof!"; }
};
