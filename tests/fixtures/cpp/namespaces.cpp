namespace math {
    double pi() { return 3.14159; }

    double square(double x) { return x * x; }
}

namespace utils {
    template<typename T>
    T max(T a, T b) {
        return a > b ? a : b;
    }
}
