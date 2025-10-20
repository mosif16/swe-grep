import Foundation

struct UserAPI {
    func fetchUser(id: Int) -> String {
        return "user_\(id)"
    }

    func mapUsers(ids: [Int]) -> [String] {
        return ids.map { fetchUser(id: $0) }
    }
}
