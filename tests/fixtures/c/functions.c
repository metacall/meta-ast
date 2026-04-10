#include <stdio.h>

int add(int a, int b) {
    return a + b;
}

void print_hello(void) {
    printf("Hello, World!\n");
}

static int counter = 0;

int get_counter(void) {
    return counter;
}
