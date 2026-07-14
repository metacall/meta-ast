function validate_input(username, password) {
    if (!username || username.length < 3) {
        return false;
    }
    if (!password || password.length < 8) {
        return false;
    }
    return true;
}

module.exports = { validate_input };
